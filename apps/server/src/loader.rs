//! 音效加载与触发入口：后台预加载 + 按需加载 + 已解码缓存。
//! Sound loading and the trigger entry point: background preload + on-demand load +
//! a cache of decoded sounds.
//!
//! 音效库可能指向很大的文件夹，全部解码进内存不现实（3 分钟立体声 ≈ 66 MB）。因此：
//! A library may point at a large folder; decoding everything into memory isn't
//! realistic (3 min stereo ≈ 66 MB). So:
//! - 加载线程按顺序预加载，累计解码量超过 [`PRELOAD_BUDGET_BYTES`] 后停止预加载；
//!   The loader thread preloads in order and stops once the decoded total exceeds
//!   [`PRELOAD_BUDGET_BYTES`];
//! - 没加载的音效第一次触发时再解码，解码完立刻播放；触发请求优先于预加载。
//!   An unloaded sound is decoded on its first trigger and then played; trigger
//!   requests take priority over preloading.
//! - 已解码的音效在控制侧留一份缓存（与音频线程共享 PCM，不额外占内存）：切换输出设备、
//!   重建音频线程时直接重新装入，不必重新解码；移除时缓存晚一秒释放，保证大块内存不在
//!   实时线程里释放。
//!   Decoded sounds are cached on the control side (sharing PCM with the audio thread,
//!   no extra memory): re-installed without re-decoding when the output device changes;
//!   on removal the cache entry is released a second later, so big buffers are never
//!   freed on the real-time thread.
//!
//! [`Player::trigger`] 是唯一的触发入口，Web API 和全局热键都走它。
//! [`Player::trigger`] is the single trigger entry point, used by the web API and
//! global hotkeys.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{select, unbounded, Receiver, Sender};
use soundboard::{BoardCommand, LoadedSound, SoundId};

use crate::library::Library;

/// 预加载的解码总量上限（字节，f32 PCM）。Preload budget (bytes of decoded f32 PCM).
pub const PRELOAD_BUDGET_BYTES: usize = 768 * 1024 * 1024;

/// 移除的音效在缓存里多留多久（等音频线程先放掉它的引用）。
/// How long removed sounds linger (so the audio thread drops its reference first).
const RELEASE_DELAY: Duration = Duration::from_secs(1);

/// 加载任务。A load job.
enum Job {
    /// 预加载（预算用完则跳过）。Preload (skipped once the budget is spent).
    Preload(SoundId),
    /// 加载后立刻播放（不受预算限制）。Load, then play (ignores the budget).
    LoadAndTrigger(SoundId),
}

/// 控制侧共享状态。Shared control-side state.
#[derive(Default)]
struct Shared {
    /// 已交给音频线程的音效（共享 PCM）。Sounds handed to the audio thread (shared PCM).
    cache: HashMap<SoundId, LoadedSound>,
    /// 待释放的音效。Sounds waiting to be released.
    graveyard: Vec<(Instant, LoadedSound)>,
    /// 已解码总量。Total decoded bytes.
    decoded_bytes: usize,
}

impl Shared {
    fn retire(&mut self, id: SoundId) -> bool {
        match self.cache.remove(&id) {
            Some(s) => {
                self.decoded_bytes = self.decoded_bytes.saturating_sub(s.decoded_bytes());
                self.graveyard.push((Instant::now(), s));
                true
            }
            None => false,
        }
    }

    fn purge(&mut self) {
        self.graveyard.retain(|(t, _)| t.elapsed() < RELEASE_DELAY);
    }
}

/// 播放控制入口。Playback control entry point.
pub struct Player {
    cmd_tx: Sender<BoardCommand>,
    urgent_tx: Sender<Job>,
    preload_tx: Sender<Job>,
    library: Arc<Library>,
    shared: Arc<Mutex<Shared>>,
}

impl Player {
    /// 创建并启动加载线程。Create the player and start the loader thread.
    pub fn new(cmd_tx: Sender<BoardCommand>, library: Arc<Library>) -> Arc<Self> {
        let (urgent_tx, urgent_rx) = unbounded();
        let (preload_tx, preload_rx) = unbounded();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker = Worker {
            cmd_tx: cmd_tx.clone(),
            library: library.clone(),
            shared: shared.clone(),
        };
        std::thread::Builder::new()
            .name("soundbridge-loader".into())
            .spawn(move || worker.run(urgent_rx, preload_rx))
            .expect("spawn loader thread");
        Arc::new(Self {
            cmd_tx,
            urgent_tx,
            preload_tx,
            library,
            shared,
        })
    }

    pub fn is_loaded(&self, id: SoundId) -> bool {
        self.shared.lock().unwrap().cache.contains_key(&id)
    }

    /// 触发音效：已加载则立刻播放，否则加载后播放。
    /// Trigger a sound: plays immediately if loaded, otherwise after loading.
    pub fn trigger(&self, id: SoundId) {
        if self.is_loaded(id) {
            let _ = self.cmd_tx.send(BoardCommand::Trigger { sound_id: id });
        } else {
            let _ = self.urgent_tx.send(Job::LoadAndTrigger(id));
        }
    }

    /// 直接装入一个已解码的音效（导入时已解码过，用于校验文件）。
    /// Install an already-decoded sound (imports decode once to validate the file).
    pub fn install(&self, loaded: LoadedSound) {
        install(&self.cmd_tx, &self.library, &self.shared, loaded);
    }

    /// 排队预加载。Queue a preload.
    pub fn preload(&self, id: SoundId) {
        let _ = self.preload_tx.send(Job::Preload(id));
    }

    /// 从音频线程卸载音效（删除、移出音效库时）。Unload from the audio thread.
    pub fn unload(&self, id: SoundId) {
        if self.shared.lock().unwrap().retire(id) {
            let _ = self.cmd_tx.send(BoardCommand::RemoveSound { sound_id: id });
        }
    }

    /// 卸载全部（切换音效库根目录时）。Unload everything (when switching the library root).
    pub fn unload_all(&self) {
        let mut sh = self.shared.lock().unwrap();
        let ids: Vec<SoundId> = sh.cache.keys().copied().collect();
        for id in ids {
            sh.retire(id);
            let _ = self.cmd_tx.send(BoardCommand::RemoveSound { sound_id: id });
        }
    }

    /// 把缓存里的音效重新装入音频线程（输出设备切换后，新的音效板是空的）。
    /// 每个音效都用音效库里的最新设置。
    /// Re-install cached sounds into the audio thread (after an output switch the new
    /// board is empty), each with the latest settings from the library.
    pub fn reinstall_all(&self) {
        let sh = self.shared.lock().unwrap();
        for (id, cached) in &sh.cache {
            let spec = self
                .library
                .spec(*id)
                .unwrap_or_else(|| cached.spec.clone());
            let _ = self
                .cmd_tx
                .send(BoardCommand::AddSound(Box::new(cached.with_spec(spec))));
        }
    }

    /// 发给音频线程的其它命令（停止、改参数、定位）。只对已加载的音效有意义，未加载的
    /// 音效下次加载时会从音效库读到最新设置。
    /// Other commands for the audio thread (stop, parameters, seek). Only meaningful
    /// for loaded sounds; unloaded ones pick up the latest settings when loaded.
    pub fn send(&self, cmd: BoardCommand) {
        let _ = self.cmd_tx.send(cmd);
    }
}

/// 装入音效：记运行时信息、放进缓存、交给音频线程。
/// Install a sound: record runtime info, cache it, hand it to the audio thread.
fn install(
    cmd_tx: &Sender<BoardCommand>,
    library: &Library,
    shared: &Mutex<Shared>,
    loaded: LoadedSound,
) {
    let id = loaded.spec.id;
    let duration = loaded.duration_secs();
    library.set_runtime(id, |rt| {
        rt.duration_secs = Some(duration);
        rt.error = None;
        rt.loaded = true;
    });
    {
        let mut sh = shared.lock().unwrap();
        sh.decoded_bytes += loaded.decoded_bytes();
        if let Some(old) = sh.cache.insert(id, loaded.clone()) {
            sh.decoded_bytes = sh.decoded_bytes.saturating_sub(old.decoded_bytes());
            sh.graveyard.push((Instant::now(), old));
        }
    }
    let _ = cmd_tx.send(BoardCommand::AddSound(Box::new(loaded)));
}

/// 加载线程。The loader thread.
struct Worker {
    cmd_tx: Sender<BoardCommand>,
    library: Arc<Library>,
    shared: Arc<Mutex<Shared>>,
}

impl Worker {
    fn run(self, urgent: Receiver<Job>, preload: Receiver<Job>) {
        loop {
            // 优先处理触发请求；空闲时每秒清理一次待释放的音效。
            // Trigger requests first; when idle, purge released sounds every second.
            let job = match urgent.try_recv() {
                Ok(job) => Some(job),
                Err(_) => select! {
                    recv(urgent) -> j => match j { Ok(j) => Some(j), Err(_) => return },
                    recv(preload) -> j => match j { Ok(j) => Some(j), Err(_) => return },
                    default(RELEASE_DELAY) => None,
                },
            };
            self.shared.lock().unwrap().purge();
            if let Some(job) = job {
                self.handle(job);
            }
        }
    }

    fn handle(&self, job: Job) {
        let (id, then_trigger) = match job {
            Job::Preload(id) => (id, false),
            Job::LoadAndTrigger(id) => (id, true),
        };
        let (loaded, over_budget) = {
            let sh = self.shared.lock().unwrap();
            (
                sh.cache.contains_key(&id),
                sh.decoded_bytes >= PRELOAD_BUDGET_BYTES,
            )
        };
        if loaded {
            if then_trigger {
                let _ = self.cmd_tx.send(BoardCommand::Trigger { sound_id: id });
            }
            return;
        }
        if !then_trigger && over_budget {
            return; // 预算用完：首次播放时再加载。Budget spent: load on first play.
        }
        // 每次都从音效库取最新定义（可能刚被改名、改设置或删除）。
        // Always fetch the latest definition (it may have just been renamed, edited or deleted).
        let Some(spec) = self.library.spec(id) else {
            return;
        };
        match LoadedSound::load(spec) {
            Ok(loaded) => {
                install(&self.cmd_tx, &self.library, &self.shared, loaded);
                if then_trigger {
                    let _ = self.cmd_tx.send(BoardCommand::Trigger { sound_id: id });
                }
            }
            Err(e) => {
                log::warn!("音效 #{id} 加载失败 failed to load sound: {e}");
                self.library.set_runtime(id, |rt| {
                    rt.error = Some(trf!(
                        "无法加载音频文件：{e}",
                        "Can't load the audio file: {e}"
                    ));
                    rt.loaded = false;
                });
            }
        }
    }
}
