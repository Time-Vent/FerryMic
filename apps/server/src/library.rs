//! 音效库：直接对应磁盘上的一个文件夹。
//! Sound library: maps directly onto a folder on disk.
//!
//! - 根目录下的音频文件就是音效，子文件夹就是分类（可多层）。在资源管理器里增删文件，
//!   重新扫描后即可同步；在 App 里新建、重命名、移动也会直接改磁盘。
//!   Audio files under the root are sounds; subfolders are categories (nested). Files
//!   added or removed in a file manager sync on rescan; creating, renaming and moving in
//!   the app changes the disk directly.
//! - 每个音效的设置（触发方式、音量、倍速、快捷键）存在根目录的 `.soundbridge.json`，
//!   按相对路径索引，音效库文件夹因此可以整体搬走或同步。
//!   Per-sound settings (trigger mode, volume, speed, hotkey) live in `.soundbridge.json`
//!   in the root, keyed by relative path, so the library folder is self-contained.
//! - 音效 id 记录在设置里，重启后不变（快捷键靠它绑定）；新文件分配新 id，id 不复用。
//!   Sound ids are stored with the settings and survive restarts (hotkeys bind to them);
//!   new files get new ids, and ids are never reused.
//!
//! 只在控制侧使用（Web API、扫描、加载线程），实时线程不接触。
//! Control side only (web API, scanning, loader thread); never the real-time thread.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use soundboard::{PlayMode, SoundId, SoundSpec, TriggerMode};

/// 支持的音频扩展名（与解码器能力一致）。Supported audio extensions (match the decoders).
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac"];

/// 根目录下的设置文件名。Settings file name in the root.
const META_FILE: &str = ".soundbridge.json";
/// 扫描的最大目录深度。Max scan depth.
const MAX_DEPTH: usize = 8;
/// 扫描的最大文件数（防止误选整个磁盘）。Max files scanned (guards against picking a whole drive).
const MAX_FILES: usize = 10_000;

type LibResult<T> = Result<T, String>;

/// 一个音效的设置（不含文件名，文件名即名称）。A sound's settings (the file name is its name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundMeta {
    pub id: SoundId,
    /// 播放方式（循环 / 独占）。Play mode (loop / exclusive).
    #[serde(default)]
    pub mode: PlayMode,
    /// 旧版字段，读取时换算成 `mode`，不再写回。Legacy field: converted to `mode` on load,
    /// never written back.
    #[serde(default, skip_serializing)]
    trigger_mode: Option<TriggerMode>,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub hotkey: Option<String>,
}

fn default_speed() -> f32 {
    1.0
}

/// 新音效的初始设置（设置里可改）。默认独占、单次、0 dB、1.0×。
/// Initial settings for new sounds (editable in Settings). Defaults: exclusive, play
/// once, 0 dB, 1.0×.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundDefaults {
    pub looping: bool,
    pub exclusive: bool,
    pub gain_db: f32,
    pub speed: f32,
}

impl Default for SoundDefaults {
    fn default() -> Self {
        Self {
            looping: false,
            exclusive: true,
            gain_db: 0.0,
            speed: 1.0,
        }
    }
}

impl SoundDefaults {
    /// 限制到界面滑块的范围。Clamp to the UI slider ranges.
    pub fn clamped(self) -> Self {
        Self {
            gain_db: if self.gain_db.is_finite() {
                self.gain_db.clamp(-30.0, 12.0)
            } else {
                0.0
            },
            speed: if self.speed.is_finite() {
                self.speed.clamp(0.5, 2.0)
            } else {
                1.0
            },
            ..self
        }
    }
}

impl SoundMeta {
    fn new(id: SoundId, d: &SoundDefaults) -> Self {
        Self {
            id,
            mode: PlayMode {
                looping: d.looping,
                exclusive: d.exclusive,
            },
            trigger_mode: None,
            gain_db: d.gain_db,
            speed: d.speed,
            hotkey: None,
        }
    }

    /// 把旧版 `trigger_mode` 换算成 `mode`。Convert a legacy `trigger_mode` into `mode`.
    fn upgrade(&mut self) {
        if let Some(tm) = self.trigger_mode.take() {
            self.mode = tm.into();
        }
    }
}

/// `.soundbridge.json` 的格式。Format of `.soundbridge.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct MetaFile {
    version: u32,
    next_id: SoundId,
    /// 相对路径（`/` 分隔）→ 设置。Relative path (`/`-separated) → settings.
    sounds: BTreeMap<String, SoundMeta>,
}

/// 库里的一个音效。A sound in the library.
#[derive(Debug, Clone)]
pub struct Entry {
    /// 相对根目录的路径，`/` 分隔，如 `游戏/欢呼.mp3`。Path relative to the root, `/`-separated.
    pub rel: String,
    pub meta: SoundMeta,
}

impl Entry {
    /// 所在文件夹（相对路径，根目录为空串）。Containing folder (relative; root is "").
    pub fn folder(&self) -> &str {
        self.rel.rsplit_once('/').map(|(f, _)| f).unwrap_or("")
    }

    /// 文件名（含扩展名）。File name (with extension).
    pub fn file_name(&self) -> &str {
        self.rel
            .rsplit_once('/')
            .map(|(_, n)| n)
            .unwrap_or(&self.rel)
    }

    /// 显示名称 = 文件名去掉扩展名。Display name = file name without extension.
    pub fn name(&self) -> &str {
        let f = self.file_name();
        f.rsplit_once('.').map(|(n, _)| n).unwrap_or(f)
    }
}

/// 扫描前后的变化。Changes found by a scan.
#[derive(Debug, Default, PartialEq)]
pub struct ScanDiff {
    pub added: Vec<SoundId>,
    pub removed: Vec<SoundId>,
}

/// 音效的运行时信息（不落盘）。Runtime info of a sound (not persisted).
#[derive(Debug, Clone, Default)]
pub struct SoundRuntime {
    /// 时长（秒），加载后才知道。Duration in seconds, known once loaded.
    pub duration_secs: Option<f32>,
    /// 加载失败原因。Load failure reason.
    pub error: Option<String>,
    /// 已解码进内存。Decoded into memory.
    pub loaded: bool,
}

struct State {
    root: PathBuf,
    meta: MetaFile,
    /// 按相对路径排序。Sorted by relative path.
    entries: Vec<Entry>,
    folders: BTreeSet<String>,
    /// 新发现的音效文件用的初始设置。Initial settings for newly found sound files.
    defaults: SoundDefaults,
}

/// 音效库。Sound library.
pub struct Library {
    state: Mutex<State>,
    runtime: Mutex<HashMap<SoundId, SoundRuntime>>,
}

impl Library {
    /// 打开（必要时创建）根目录并扫描。Open (creating if needed) the root and scan it.
    #[cfg(test)]
    pub fn open(root: PathBuf) -> Self {
        Self::open_with(root, SoundDefaults::default())
    }

    /// 打开并扫描；新发现的音效用 `defaults` 作为初始设置。
    /// Open and scan; newly found sounds start with `defaults`.
    pub fn open_with(root: PathBuf, defaults: SoundDefaults) -> Self {
        let _ = std::fs::create_dir_all(&root);
        let meta = load_meta(&root);
        let mut state = State {
            root,
            meta,
            entries: Vec::new(),
            folders: BTreeSet::new(),
            defaults,
        };
        scan(&mut state);
        Self {
            state: Mutex::new(state),
            runtime: Mutex::new(HashMap::new()),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.state.lock().unwrap().root.clone()
    }

    /// 修改新音效的初始设置（已有音效不变）。Change the initial settings for new sounds
    /// (existing sounds are unchanged).
    pub fn set_defaults(&self, defaults: SoundDefaults) {
        self.state.lock().unwrap().defaults = defaults;
    }

    /// 重新扫描磁盘。Rescan the disk.
    pub fn rescan(&self) -> ScanDiff {
        let diff = scan(&mut self.state.lock().unwrap());
        let mut rt = self.runtime.lock().unwrap();
        for id in &diff.removed {
            rt.remove(id);
        }
        diff
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.state.lock().unwrap().entries.clone()
    }

    /// 所有文件夹（相对路径，已排序，含空文件夹）。All folders (relative, sorted, incl. empty).
    pub fn folders(&self) -> Vec<String> {
        self.state.lock().unwrap().folders.iter().cloned().collect()
    }

    pub fn entry(&self, id: SoundId) -> Option<Entry> {
        let s = self.state.lock().unwrap();
        s.entries.iter().find(|e| e.meta.id == id).cloned()
    }

    /// 音效的完整定义（供解码 / 音频线程）。The full definition (for decoding / audio).
    pub fn spec(&self, id: SoundId) -> Option<SoundSpec> {
        let s = self.state.lock().unwrap();
        let e = s.entries.iter().find(|e| e.meta.id == id)?;
        Some(to_spec(&s.root, e))
    }

    /// 修改音效设置并保存。Modify a sound's settings and save.
    pub fn update_meta(&self, id: SoundId, f: impl FnOnce(&mut SoundMeta)) -> Option<SoundMeta> {
        let mut s = self.state.lock().unwrap();
        let rel = s.entries.iter().find(|e| e.meta.id == id)?.rel.clone();
        let meta = s.meta.sounds.get_mut(&rel)?;
        f(meta);
        meta.id = id; // id 不可改。The id is immutable.
        let updated = meta.clone();
        if let Some(e) = s.entries.iter_mut().find(|e| e.meta.id == id) {
            e.meta = updated.clone();
        }
        save_meta(&s.root, &s.meta);
        Some(updated)
    }

    /// 新建文件夹，返回清理后的相对路径。Create a folder; returns the cleaned relative path.
    pub fn create_folder(&self, rel: &str) -> LibResult<String> {
        let rel = clean_rel(rel)?;
        let mut s = self.state.lock().unwrap();
        std::fs::create_dir_all(s.root.join(&rel))
            .map_err(|e| trf!("无法创建文件夹：{e}", "Can't create the folder: {e}"))?;
        scan(&mut s);
        Ok(rel)
    }

    /// 重命名 / 移动文件夹（`to` 为新的完整相对路径）。音效设置随之迁移。
    /// Rename / move a folder (`to` is the new full relative path). Settings follow.
    pub fn rename_folder(&self, from: &str, to: &str) -> LibResult<String> {
        let from = clean_rel(from)?;
        let to = clean_rel(to)?;
        if from == to {
            return Ok(to);
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(tr!(
                "不能把文件夹移动到它自己的子文件夹里",
                "Can't move a folder into its own subfolder"
            )
            .into());
        }
        let mut s = self.state.lock().unwrap();
        let (src, dst) = (s.root.join(&from), s.root.join(&to));
        if !src.is_dir() {
            return Err(tr!("文件夹不存在", "Folder not found").into());
        }
        if dst.exists() {
            return Err(tr!(
                "目标位置已有同名文件夹",
                "A folder with that name already exists"
            )
            .into());
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&src, &dst)
            .map_err(|e| trf!("无法移动文件夹：{e}", "Can't move the folder: {e}"))?;
        rekey_prefix(&mut s.meta, &format!("{from}/"), &format!("{to}/"));
        scan(&mut s);
        Ok(to)
    }

    /// 删除文件夹：只允许删除不含音效的文件夹（防止误删）。
    /// Delete a folder: only folders without sounds may be deleted (prevents accidents).
    pub fn delete_folder(&self, rel: &str) -> LibResult<()> {
        let rel = clean_rel(rel)?;
        let mut s = self.state.lock().unwrap();
        let prefix = format!("{rel}/");
        if s.entries.iter().any(|e| e.rel.starts_with(&prefix)) {
            return Err(tr!(
                "文件夹里还有音效，请先移走或删除",
                "The folder still has sounds; move or delete them first"
            )
            .into());
        }
        let dir = s.root.join(&rel);
        move_to_trash(&dir)
            .map_err(|e| trf!("无法删除文件夹：{e}", "Can't delete the folder: {e}"))?;
        scan(&mut s);
        Ok(())
    }

    /// 导入文件到 `folder`（重名时自动加编号），返回新音效的 id。
    /// Import a file into `folder` (numbered on name clash); returns the new sound's id.
    pub fn import(&self, folder: &str, file_name: &str, bytes: &[u8]) -> LibResult<SoundId> {
        let folder = clean_folder(folder)?;
        let name = sanitize_file_name(file_name);
        let (stem, ext) = split_ext(&name);
        if !is_audio_ext(ext) {
            return Err(trf!(
                "不支持的文件类型「{name}」，支持 {}",
                "Unsupported file type \"{name}\"; supported: {}",
                AUDIO_EXTENSIONS.join(" / ")
            ));
        }
        let mut s = self.state.lock().unwrap();
        let dir = s.root.join(&folder);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = unique_path(&dir, stem, ext);
        std::fs::write(&path, bytes)
            .map_err(|e| trf!("无法写入文件：{e}", "Can't write the file: {e}"))?;
        let rel = join_rel(&folder, &path.file_name().unwrap().to_string_lossy());
        scan(&mut s);
        s.entries
            .iter()
            .find(|e| e.rel == rel)
            .map(|e| e.meta.id)
            .ok_or_else(|| tr!("导入后未找到文件", "File not found after import").into())
    }

    /// 重命名音效（改文件名，保留扩展名）。Rename a sound (renames the file, keeps the extension).
    pub fn rename_sound(&self, id: SoundId, new_name: &str) -> LibResult<()> {
        let new_name = sanitize_file_name(new_name);
        let mut s = self.state.lock().unwrap();
        let e = find(&s, id)?;
        let ext = split_ext(e.file_name()).1.to_string();
        let new_rel = join_rel(e.folder(), &format!("{new_name}.{ext}"));
        move_entry(&mut s, &e.rel, &new_rel)
    }

    /// 把音效移动到另一个文件夹（`""` = 根目录）。Move a sound to another folder ("" = root).
    pub fn move_sound(&self, id: SoundId, folder: &str) -> LibResult<()> {
        let folder = clean_folder(folder)?;
        let mut s = self.state.lock().unwrap();
        let e = find(&s, id)?;
        let new_rel = join_rel(&folder, e.file_name());
        move_entry(&mut s, &e.rel, &new_rel)
    }

    /// 删除音效：文件移到回收站（失败则直接删除）。Delete a sound: file goes to the
    /// recycle bin (deleted outright if that fails).
    pub fn delete_sound(&self, id: SoundId) -> LibResult<Entry> {
        let mut s = self.state.lock().unwrap();
        let e = find(&s, id)?;
        let path = s.root.join(&e.rel);
        move_to_trash(&path)
            .map_err(|err| trf!("无法删除文件：{err}", "Can't delete the file: {err}"))?;
        s.meta.sounds.remove(&e.rel);
        scan(&mut s);
        self.runtime.lock().unwrap().remove(&id);
        Ok(e)
    }

    /// 切换根目录；`move_contents` 为真时把当前库里的所有内容移过去。
    /// Switch the root; with `move_contents`, everything in the current library moves over.
    pub fn set_root(&self, new_root: PathBuf, move_contents: bool) -> LibResult<()> {
        let mut s = self.state.lock().unwrap();
        if new_root == s.root {
            return Ok(());
        }
        if new_root.starts_with(&s.root) && move_contents {
            return Err(tr!(
                "不能把音效库移动到它自己的子文件夹里",
                "Can't move the library into its own subfolder"
            )
            .into());
        }
        std::fs::create_dir_all(&new_root)
            .map_err(|e| trf!("无法创建文件夹：{e}", "Can't create the folder: {e}"))?;
        if move_contents {
            move_dir_contents(&s.root, &new_root)
                .map_err(|e| trf!("移动文件失败：{e}", "Moving files failed: {e}"))?;
        }
        s.root = new_root;
        s.meta = load_meta(&s.root);
        scan(&mut s);
        self.runtime.lock().unwrap().clear();
        Ok(())
    }

    pub fn set_runtime(&self, id: SoundId, f: impl FnOnce(&mut SoundRuntime)) {
        f(self.runtime.lock().unwrap().entry(id).or_default());
    }

    pub fn runtime(&self, id: SoundId) -> SoundRuntime {
        self.runtime
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }
}

fn find(s: &State, id: SoundId) -> LibResult<Entry> {
    s.entries
        .iter()
        .find(|e| e.meta.id == id)
        .cloned()
        .ok_or_else(|| tr!("音效不存在", "Sound not found").into())
}

fn to_spec(root: &Path, e: &Entry) -> SoundSpec {
    SoundSpec {
        id: e.meta.id,
        name: e.name().to_string(),
        file_path: root.join(&e.rel),
        mode: e.meta.mode,
        gain_db: e.meta.gain_db,
        speed: e.meta.speed,
        hotkey: e.meta.hotkey.clone(),
    }
}

/// 移动/重命名一个音效文件，设置随之迁移。Move/rename a sound file; its settings follow.
fn move_entry(s: &mut State, from: &str, to: &str) -> LibResult<()> {
    if from == to {
        return Ok(());
    }
    let (src, dst) = (s.root.join(from), s.root.join(to));
    // Windows 大小写不敏感：只改大小写时目标「已存在」但就是自己。
    // Windows is case-insensitive: a case-only rename "exists" but is the same file.
    if dst.exists() && !from.eq_ignore_ascii_case(to) {
        return Err(tr!(
            "目标位置已有同名文件",
            "A file with that name already exists"
        )
        .into());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&src, &dst)
        .map_err(|e| trf!("无法移动文件：{e}", "Can't move the file: {e}"))?;
    if let Some(meta) = s.meta.sounds.remove(from) {
        s.meta.sounds.insert(to.to_string(), meta);
    }
    scan(s);
    Ok(())
}

/// 扫描根目录，同步条目与设置，并保存设置文件。Scan the root, sync entries and settings,
/// and save the settings file.
fn scan(s: &mut State) -> ScanDiff {
    let mut files = Vec::new();
    let mut folders = BTreeSet::new();
    walk(&s.root, "", 0, &mut files, &mut folders);
    files.sort();

    let old_ids: HashSet<SoundId> = s.entries.iter().map(|e| e.meta.id).collect();
    let mut used = HashSet::new();
    let mut entries = Vec::with_capacity(files.len());
    let mut metas = BTreeMap::new();
    let max_existing = s.meta.sounds.values().map(|m| m.id).max().unwrap_or(0);
    s.meta.next_id = s.meta.next_id.max(max_existing + 1).max(1);
    for rel in files {
        let mut meta = s.meta.sounds.get(&rel).cloned().unwrap_or_else(|| {
            let id = s.meta.next_id;
            s.meta.next_id += 1;
            SoundMeta::new(id, &s.defaults)
        });
        // 手改文件导致 id 重复时重新分配。Reassign duplicated ids (hand-edited files).
        if !used.insert(meta.id) {
            meta.id = s.meta.next_id;
            s.meta.next_id += 1;
            used.insert(meta.id);
        }
        metas.insert(rel.clone(), meta.clone());
        entries.push(Entry { rel, meta });
    }
    let new_ids: HashSet<SoundId> = entries.iter().map(|e| e.meta.id).collect();
    let mut diff = ScanDiff {
        added: new_ids.difference(&old_ids).copied().collect(),
        removed: old_ids.difference(&new_ids).copied().collect(),
    };
    diff.added.sort_unstable();
    diff.removed.sort_unstable();

    s.meta.version = 2;
    s.meta.sounds = metas;
    s.entries = entries;
    s.folders = folders;
    save_meta(&s.root, &s.meta);
    diff
}

/// 递归收集音频文件与文件夹（跳过以 `.` 开头的项）。Recursively collect audio files and
/// folders (skipping entries starting with `.`).
fn walk(
    dir: &Path,
    prefix: &str,
    depth: usize,
    files: &mut Vec<String>,
    folders: &mut BTreeSet<String>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for item in rd.flatten() {
        if files.len() >= MAX_FILES {
            return;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let rel = join_rel(prefix, &name);
        let Ok(ft) = item.file_type() else { continue };
        if ft.is_dir() {
            folders.insert(rel.clone());
            if depth + 1 < MAX_DEPTH {
                walk(&item.path(), &rel, depth + 1, files, folders);
            }
        } else if ft.is_file() && is_audio_ext(split_ext(&name).1) {
            files.push(rel);
        }
    }
}

fn load_meta(root: &Path) -> MetaFile {
    let mut meta: MetaFile = std::fs::read_to_string(root.join(META_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for m in meta.sounds.values_mut() {
        m.upgrade();
    }
    meta
}

/// 写设置文件（临时文件 + 改名）。失败只记警告。Write the settings file (temp + rename).
fn save_meta(root: &Path, meta: &MetaFile) {
    let path = root.join(META_FILE);
    let result = (|| -> std::io::Result<()> {
        let tmp = root.join(format!("{META_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_string_pretty(meta)?)?;
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = result {
        log::warn!("保存音效设置失败 failed to save {}: {e}", path.display());
    }
}

/// 把 `from` 前缀的键改成 `to` 前缀。Rename keys starting with `from` to start with `to`.
fn rekey_prefix(meta: &mut MetaFile, from: &str, to: &str) {
    let keys: Vec<String> = meta
        .sounds
        .keys()
        .filter(|k| k.starts_with(from))
        .cloned()
        .collect();
    for k in keys {
        if let Some(m) = meta.sounds.remove(&k) {
            meta.sounds.insert(format!("{to}{}", &k[from.len()..]), m);
        }
    }
}

/// 移到回收站；不支持时（如网络盘）直接删除。Move to the recycle bin; delete outright
/// where unsupported (e.g. network drives).
fn move_to_trash(path: &Path) -> std::io::Result<()> {
    if trash::delete(path).is_ok() {
        return Ok(());
    }
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// 把 `from` 目录里的所有内容移到 `to`（跨磁盘时复制后删除）。
/// Move everything in `from` into `to` (copy + delete across drives).
fn move_dir_contents(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(rd) = std::fs::read_dir(from) else {
        return Ok(()); // 旧目录不存在：没东西可移。Old dir missing: nothing to move.
    };
    for item in rd.flatten() {
        let src = item.path();
        let dst = to.join(item.file_name());
        if dst.exists() {
            if item.file_name() == META_FILE {
                continue; // 目标已有设置文件：保留目标的。Keep the target's settings file.
            }
            return Err(std::io::Error::other(trf!(
                "目标位置已有「{}」",
                "\"{}\" already exists at the destination",
                item.file_name().to_string_lossy()
            )));
        }
        if std::fs::rename(&src, &dst).is_err() {
            copy_recursive(&src, &dst)?;
            if src.is_dir() {
                std::fs::remove_dir_all(&src)?;
            } else {
                std::fs::remove_file(&src)?;
            }
        }
    }
    Ok(())
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for item in std::fs::read_dir(src)?.flatten() {
            copy_recursive(&item.path(), &dst.join(item.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// 路径工具 Path helpers
// ---------------------------------------------------------------------------

pub fn is_audio_ext(ext: &str) -> bool {
    AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

/// 拆成（主名, 扩展名）。Split into (stem, extension).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (name, ""),
    }
}

fn join_rel(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// `dir` 下不重名的路径：`名称.ext`、`名称 (2).ext`……
/// A non-clashing path in `dir`: `name.ext`, `name (2).ext`, …
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// 清理相对路径：拆分 `/` `\`，去掉空段与 `.`/`..`，逐段清理非法字符。不允许为空。
/// Clean a relative path: split on `/` `\`, drop empty / `.` / `..` segments, sanitize
/// each segment. Must not be empty.
pub fn clean_rel(input: &str) -> LibResult<String> {
    let parts: Vec<String> = input
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .map(sanitize_file_name)
        .collect();
    if parts.is_empty() {
        return Err(tr!("名称不能为空", "Name can't be empty").into());
    }
    Ok(parts.join("/"))
}

/// 同 [`clean_rel`]，但允许空（表示根目录）。Like [`clean_rel`] but empty means the root.
fn clean_folder(input: &str) -> LibResult<String> {
    if input.trim().trim_matches(['/', '\\']).is_empty() {
        Ok(String::new())
    } else {
        clean_rel(input)
    }
}

/// 清理文件名：只保留最后一段，替换 Windows 不允许的字符。
/// Sanitize a file name: keep only the final component, replace characters Windows
/// disallows.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "sound".into()
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// 旧版音效库迁移 Migrating the old library
// ---------------------------------------------------------------------------

/// 旧版（v1）`library.json`：音效列表 + 绝对路径。Old (v1) `library.json`: list + absolute paths.
#[derive(Deserialize)]
struct LibraryV1 {
    #[serde(default)]
    sounds: Vec<SoundV1>,
}

/// 旧版音效定义。An old (v1) sound definition.
#[derive(Deserialize)]
struct SoundV1 {
    id: SoundId,
    name: String,
    file_path: PathBuf,
    trigger_mode: TriggerMode,
    #[serde(default)]
    gain_db: f32,
    #[serde(default = "default_speed")]
    speed: f32,
    #[serde(default)]
    hotkey: Option<String>,
}

/// 把数据目录里的旧版 `library.json` 迁移到新音效库：文件改名为「名称.扩展名」放进根目录，
/// 设置（含 id、快捷键）保留；原 `library.json` 改名为 `library.v1.json.bak`。
/// Migrate an old `library.json` from the data dir into the new library: files are
/// renamed to "name.ext" in the root, settings (incl. id and hotkey) are kept, and the
/// old file becomes `library.v1.json.bak`.
pub fn migrate_v1(data_dir: &Path, root: &Path) {
    let old = data_dir.join("library.json");
    let Ok(text) = std::fs::read_to_string(&old) else {
        return;
    };
    let Ok(v1) = serde_json::from_str::<LibraryV1>(&text) else {
        return;
    };
    let _ = std::fs::create_dir_all(root);
    let mut meta = load_meta(root);
    for spec in v1.sounds {
        let src = &spec.file_path;
        if !src.is_file() {
            continue;
        }
        let ext = src
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dst = unique_path(root, &sanitize_file_name(&spec.name), &ext);
        let moved = if src.starts_with(root) {
            std::fs::rename(src, &dst).is_ok()
        } else {
            std::fs::copy(src, &dst).is_ok()
        };
        if !moved {
            continue;
        }
        let rel = dst.file_name().unwrap().to_string_lossy().into_owned();
        meta.next_id = meta.next_id.max(spec.id + 1);
        meta.sounds.insert(
            rel,
            SoundMeta {
                id: spec.id,
                mode: spec.trigger_mode.into(),
                trigger_mode: None,
                gain_db: spec.gain_db,
                speed: spec.speed,
                hotkey: spec.hotkey,
            },
        );
    }
    save_meta(root, &meta);
    let _ = std::fs::rename(&old, data_dir.join("library.v1.json.bak"));
    log::info!(
        "已迁移旧版音效库 migrated the v1 library into {}",
        root.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn new_sounds_use_defaults_existing_keep_theirs() {
        let root = temp("sb-lib-defaults");
        touch(&root.join("old.wav"));
        // 默认：独占、单次。Default: exclusive, play once.
        let lib = Library::open(root.clone());
        let old = lib.entries()[0].meta.clone();
        assert!(old.mode.exclusive && !old.mode.looping);

        lib.set_defaults(SoundDefaults {
            looping: true,
            exclusive: false,
            gain_db: -6.0,
            speed: 1.5,
        });
        touch(&root.join("new.wav"));
        lib.rescan();
        let get = |rel: &str| {
            lib.entries()
                .into_iter()
                .find(|e| e.rel == rel)
                .unwrap()
                .meta
        };
        let new = get("new.wav");
        assert!(new.mode.looping && !new.mode.exclusive);
        assert_eq!((new.gain_db, new.speed), (-6.0, 1.5));
        // 已有音效不受影响。Existing sounds are unaffected.
        assert_eq!(get("old.wav"), old);
    }

    #[test]
    fn defaults_are_clamped() {
        let d = SoundDefaults {
            gain_db: 99.0,
            speed: f32::NAN,
            ..SoundDefaults::default()
        }
        .clamped();
        assert_eq!((d.gain_db, d.speed), (12.0, 1.0));
    }

    #[test]
    fn scans_folders_and_audio_files_only() {
        let root = temp("sb-lib-scan");
        touch(&root.join("a.wav"));
        touch(&root.join("游戏/欢呼.mp3"));
        touch(&root.join("游戏/搞笑/笑声.ogg"));
        touch(&root.join("notes.txt"));
        touch(&root.join(".hidden/x.wav"));
        std::fs::create_dir_all(root.join("空文件夹")).unwrap();

        let lib = Library::open(root.clone());
        let rels: Vec<String> = lib.entries().into_iter().map(|e| e.rel).collect();
        assert_eq!(rels, vec!["a.wav", "游戏/搞笑/笑声.ogg", "游戏/欢呼.mp3"]);
        assert_eq!(lib.folders(), vec!["游戏", "游戏/搞笑", "空文件夹"]);
        let e = lib
            .entries()
            .into_iter()
            .find(|e| e.rel == "游戏/欢呼.mp3")
            .unwrap();
        assert_eq!((e.folder(), e.name()), ("游戏", "欢呼"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ids_and_settings_survive_reopen_and_external_changes() {
        let root = temp("sb-lib-ids");
        touch(&root.join("a.wav"));
        let lib = Library::open(root.clone());
        let id = lib.entries()[0].meta.id;
        lib.update_meta(id, |m| {
            m.gain_db = -6.0;
            m.hotkey = Some("F9".into());
        });

        // 外部新增文件：重扫后出现，id 不同。File added externally: appears on rescan.
        touch(&root.join("b.wav"));
        let diff = lib.rescan();
        assert_eq!(diff.added.len(), 1);
        assert!(diff.removed.is_empty());

        // 重新打开：id 与设置不变。Reopen: ids and settings unchanged.
        let lib2 = Library::open(root.clone());
        let a = lib2
            .entries()
            .into_iter()
            .find(|e| e.rel == "a.wav")
            .unwrap();
        assert_eq!(a.meta.id, id);
        assert_eq!(a.meta.gain_db, -6.0);
        assert_eq!(a.meta.hotkey.as_deref(), Some("F9"));

        // 外部删除：重扫后移除。Deleted externally: removed on rescan.
        std::fs::remove_file(root.join("a.wav")).unwrap();
        assert_eq!(lib2.rescan().removed, vec![id]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_and_move_keep_settings() {
        let root = temp("sb-lib-move");
        touch(&root.join("a.wav"));
        let lib = Library::open(root.clone());
        let id = lib.entries()[0].meta.id;
        lib.update_meta(id, |m| m.speed = 1.5);

        lib.rename_sound(id, "鼓掌").unwrap();
        assert!(root.join("鼓掌.wav").is_file());
        lib.move_sound(id, "掌声/现场").unwrap();
        assert!(root.join("掌声/现场/鼓掌.wav").is_file());
        lib.rename_folder("掌声", "欢呼区").unwrap();
        let e = lib.entry(id).unwrap();
        assert_eq!(e.rel, "欢呼区/现场/鼓掌.wav");
        assert_eq!(e.meta.speed, 1.5);

        // 不能移进自己的子文件夹。Can't move into its own subfolder.
        assert!(lib.rename_folder("欢呼区", "欢呼区/里面").is_err());
        // 有音效的文件夹不能删。Folders with sounds can't be deleted.
        assert!(lib.delete_folder("欢呼区").is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_numbers_duplicates_and_rejects_non_audio() {
        let root = temp("sb-lib-import");
        let lib = Library::open(root.clone());
        let a = lib.import("游戏", "hit.wav", b"x").unwrap();
        let b = lib.import("游戏", "hit.wav", b"x").unwrap();
        assert_ne!(a, b);
        assert_eq!(lib.entry(b).unwrap().rel, "游戏/hit (2).wav");
        assert!(lib.import("", "notes.txt", b"x").is_err());
        // 路径穿越被清理。Path traversal is sanitized.
        let c = lib.import("../../外面", "x.wav", b"x").unwrap();
        assert_eq!(lib.entry(c).unwrap().rel, "外面/x.wav");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_root_moves_contents() {
        let base = temp("sb-lib-root");
        let (old, new) = (base.join("old"), base.join("new"));
        touch(&old.join("分类/a.wav"));
        let lib = Library::open(old.clone());
        let id = lib.entries()[0].meta.id;
        lib.update_meta(id, |m| m.gain_db = 3.0);

        lib.set_root(new.clone(), true).unwrap();
        assert!(new.join("分类/a.wav").is_file());
        let e = lib.entries().into_iter().next().unwrap();
        assert_eq!((e.meta.id, e.meta.gain_db), (id, 3.0));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn migrates_v1_library() {
        let data = temp("sb-lib-migrate");
        let root = data.join("sounds");
        touch(&root.join("3-clap.wav"));
        let v1 = format!(
            r#"{{"next_id":4,"sounds":[{{"id":3,"name":"鼓掌","file_path":{:?},"trigger_mode":"Loop","gain_db":-3.0,"speed":1.0,"hotkey":"F9"}}]}}"#,
            root.join("3-clap.wav")
        );
        std::fs::write(data.join("library.json"), v1).unwrap();

        migrate_v1(&data, &root);
        let lib = Library::open(root.clone());
        let e = lib.entries().into_iter().next().unwrap();
        assert_eq!(e.rel, "鼓掌.wav");
        assert_eq!(e.meta.id, 3);
        assert!(e.meta.mode.looping && !e.meta.mode.exclusive);
        assert_eq!(e.meta.hotkey.as_deref(), Some("F9"));
        assert!(data.join("library.v1.json.bak").is_file());

        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn upgrades_legacy_trigger_mode_in_meta_file() {
        let root = temp("sb-lib-legacy-mode");
        touch(&root.join("a.wav"));
        std::fs::write(
            root.join(META_FILE),
            r#"{"version":2,"next_id":2,"sounds":{"a.wav":{"id":1,"trigger_mode":"Exclusive"}}}"#,
        )
        .unwrap();
        let lib = Library::open(root.clone());
        let e = lib.entries().into_iter().next().unwrap();
        assert!(e.meta.mode.exclusive && !e.meta.mode.looping);
        // 写回后不再含旧字段。Rewritten without the legacy field.
        let text = std::fs::read_to_string(root.join(META_FILE)).unwrap();
        assert!(!text.contains("trigger_mode"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cleans_relative_paths() {
        assert_eq!(clean_rel("a/b").unwrap(), "a/b");
        assert_eq!(clean_rel("..\\a\\..\\b").unwrap(), "a/b");
        assert_eq!(clean_rel(" x : y ").unwrap(), "x _ y");
        assert!(clean_rel("../..").is_err());
        assert_eq!(clean_folder("/").unwrap(), "");
    }
}
