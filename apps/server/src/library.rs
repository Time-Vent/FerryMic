//! 音效库持久化：音效列表存在 `library.json`，导入的音频文件复制到 `sounds/`。
//! Sound library persistence: the list lives in `library.json`; imported audio files
//! are copied into `sounds/`.
//!
//! 只在控制侧使用（Web API、启动加载），实时线程不接触。每次修改立即写盘（先写临时
//! 文件再改名，避免写一半断电损坏）。
//! Control side only (web API, startup load); the real-time thread never touches it.
//! Every change is written immediately (temp file + rename, so a crash mid-write
//! can't corrupt it).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use soundboard::{SoundId, SoundSpec};

/// 落盘格式。On-disk format.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct LibraryData {
    /// 下一个可用 id。Next free id.
    next_id: SoundId,
    sounds: Vec<SoundSpec>,
}

/// 音效的运行时信息（不落盘）。Runtime info of a sound (not persisted).
#[derive(Debug, Clone, Default)]
pub struct SoundRuntime {
    /// 时长（秒），加载成功时有值。Duration in seconds, set when loaded.
    pub duration_secs: Option<f32>,
    /// 加载失败原因（如文件被删除）。Load failure reason (e.g. file deleted).
    pub error: Option<String>,
}

/// 音效库。Sound library.
pub struct Library {
    path: PathBuf,
    sounds_dir: PathBuf,
    data: Mutex<LibraryData>,
    runtime: Mutex<HashMap<SoundId, SoundRuntime>>,
}

impl Library {
    /// 从数据目录加载（`library.json` 不存在或损坏时为空库）。
    /// Load from the data directory (empty when `library.json` is missing or corrupt).
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("library.json");
        let mut data: LibraryData = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // next_id 至少比现有最大 id 大 1（防止手改文件后 id 冲突）。
        // next_id must exceed the largest existing id (guards against hand edits).
        let max_id = data.sounds.iter().map(|s| s.id).max().unwrap_or(0);
        data.next_id = data.next_id.max(max_id + 1);
        Self {
            path,
            sounds_dir: data_dir.join("sounds"),
            data: Mutex::new(data),
            runtime: Mutex::new(HashMap::new()),
        }
    }

    /// 导入文件存放目录。Directory for imported files.
    pub fn sounds_dir(&self) -> &Path {
        &self.sounds_dir
    }

    /// 所有音效（按添加顺序）。All sounds (in insertion order).
    pub fn list(&self) -> Vec<SoundSpec> {
        self.data.lock().unwrap().sounds.clone()
    }

    pub fn get(&self, id: SoundId) -> Option<SoundSpec> {
        self.data
            .lock()
            .unwrap()
            .sounds
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// 分配一个新 id（随下一次保存落盘）。Allocate a new id (persisted on next save).
    pub fn alloc_id(&self) -> SoundId {
        let mut data = self.data.lock().unwrap();
        let id = data.next_id;
        data.next_id += 1;
        id
    }

    /// 添加音效并保存。Add a sound and save.
    pub fn insert(&self, spec: SoundSpec) {
        let mut data = self.data.lock().unwrap();
        data.sounds.push(spec);
        self.save(&data);
    }

    /// 修改音效并保存，返回修改后的定义。Modify a sound and save; returns the result.
    pub fn update(&self, id: SoundId, f: impl FnOnce(&mut SoundSpec)) -> Option<SoundSpec> {
        let mut data = self.data.lock().unwrap();
        let spec = data.sounds.iter_mut().find(|s| s.id == id)?;
        f(spec);
        let out = spec.clone();
        self.save(&data);
        Some(out)
    }

    /// 删除音效并保存；若音频文件是导入时复制进来的，一并删除。
    /// Remove a sound and save; deletes the audio file too if it was copied in on import.
    pub fn remove(&self, id: SoundId) -> Option<SoundSpec> {
        let mut data = self.data.lock().unwrap();
        let idx = data.sounds.iter().position(|s| s.id == id)?;
        let spec = data.sounds.remove(idx);
        self.save(&data);
        drop(data);
        self.runtime.lock().unwrap().remove(&id);
        if self.owns(&spec.file_path) {
            let _ = std::fs::remove_file(&spec.file_path);
        }
        Some(spec)
    }

    /// 导入文件的目标路径：`sounds/<id>-<清理后的文件名>`。
    /// Target path for an imported file: `sounds/<id>-<sanitized name>`.
    pub fn import_path(&self, id: SoundId, original_name: &str) -> PathBuf {
        self.sounds_dir
            .join(format!("{id}-{}", sanitize_file_name(original_name)))
    }

    /// 文件是否在导入目录里（即由本程序管理）。Whether the file is in the import dir.
    pub fn owns(&self, path: &Path) -> bool {
        path.starts_with(&self.sounds_dir)
    }

    pub fn set_runtime(&self, id: SoundId, rt: SoundRuntime) {
        self.runtime.lock().unwrap().insert(id, rt);
    }

    pub fn runtime(&self, id: SoundId) -> SoundRuntime {
        self.runtime
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    /// 写盘（临时文件 + 改名）。失败只记警告。
    /// Write to disk (temp file + rename). Failures only log a warning.
    fn save(&self, data: &LibraryData) {
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string_pretty(data)?)?;
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = result {
            log::warn!(
                "保存音效库失败 failed to save library {}: {e}",
                self.path.display()
            );
        }
    }
}

/// 清理文件名：只保留文件名部分，替换 Windows 不允许的字符。
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

#[cfg(test)]
mod tests {
    use super::*;
    use soundboard::TriggerMode;

    fn spec(id: SoundId, path: PathBuf) -> SoundSpec {
        SoundSpec {
            id,
            name: format!("s{id}"),
            file_path: path,
            trigger_mode: TriggerMode::OneShot,
            gain_db: 0.0,
            speed: 1.0,
            hotkey: None,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn persists_across_reload() {
        let dir = temp("soundbridge-library-test");
        let lib = Library::load(&dir);
        let a = lib.alloc_id();
        lib.insert(spec(a, "C:/x/a.wav".into()));
        let b = lib.alloc_id();
        lib.insert(spec(b, "C:/x/b.wav".into()));
        lib.update(a, |s| s.name = "鼓掌".into());
        lib.remove(b);

        let reloaded = Library::load(&dir);
        let list = reloaded.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "鼓掌");
        // 删除过的 id 不会被复用。Removed ids are never reused.
        assert!(reloaded.alloc_id() > b);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_deletes_only_imported_files() {
        let dir = temp("soundbridge-library-owned");
        let lib = Library::load(&dir);
        std::fs::create_dir_all(lib.sounds_dir()).unwrap();

        let owned = lib.import_path(1, "clap.wav");
        std::fs::write(&owned, b"x").unwrap();
        lib.insert(spec(1, owned.clone()));
        let external = dir.join("external.wav");
        std::fs::write(&external, b"x").unwrap();
        lib.insert(spec(2, external.clone()));

        lib.remove(1);
        lib.remove(2);
        assert!(
            !owned.exists(),
            "导入的文件应被删除 imported file should be deleted"
        );
        assert!(
            external.exists(),
            "外部文件应保留 external file must be kept"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitizes_file_names() {
        assert_eq!(sanitize_file_name("C:\\a\\b\\clap.mp3"), "clap.mp3");
        assert_eq!(sanitize_file_name("../../evil.wav"), "evil.wav");
        assert_eq!(sanitize_file_name("a:b?.wav"), "a_b_.wav");
        assert_eq!(sanitize_file_name(".."), "sound");
    }
}
