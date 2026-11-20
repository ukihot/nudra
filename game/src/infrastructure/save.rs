//! 進行記録のローカル保存（オフラインファースト、基本設計 5.7）。
//!
//! ネットワーク・ユーザー認証・外部サーバーに依存しない。ローカルのファイルが唯一の正本で、
//! 将来のオンライン機能は、このファイルを同期するサービスとして後から足す。
//!
//! ファイル形式（リトルエンディアン）:
//! ```text
//! "NUDRASAV"(8) | 版番号 u16 | 本体の長さ u32 | 本体のCRC32 u32 | 本体（postcard）
//! ```
//! - 版ごとの保存形式（`SaveV1` …）はリリース後に変更しない。変更は新しい版を足して移行する。
//! - 保存は「一時ファイルへ書いて同期 → 現行を `.bak` へ退避 → 一時ファイルを現行へ」の順。
//! - 読めない現行ファイルは `.corrupt` として隔離し、`.bak` から復旧する。
//! - 未来の版のファイルは読み取り専用とし、上書きしない（旧版のゲームでデータを壊さないため）。

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use bevy::{log::warn, prelude::Resource};
use domain::{
    difficulty::Difficulty,
    id::{PusherCharacterId, StageId},
    progress::{Language, PlayerProgress, ProgressError, RenderQuality},
    repository::ProgressRepository,
};
use serde::{Deserialize, Serialize};

const MAGIC: [u8; 8] = *b"NUDRASAV";
pub const CURRENT_VERSION: u16 = 3;
const HEADER_LEN: usize = MAGIC.len() + 2 + 4 + 4;
/// 本体の上限。壊れた長さで巨大な確保をしないため。
const MAX_PAYLOAD: usize = 1 << 20;
const FILE_NAME: &str = "progress.sav";

// ── 版ごとの保存形式（リリース後は変更しない） ───────────────

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SaveV1 {
    last_character: Option<String>,
    games_played: u32,
    records: Vec<StageRecordV1>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct StageRecordV1 {
    stage: String,
    plays: u32,
    best_defeated: u32,
}

#[derive(Serialize, Deserialize, Debug)]
struct SaveV2 {
    progress: SaveV1,
    language: SavedLanguage,
    clears: Vec<(String, u8)>,
}

#[derive(Serialize, Deserialize, Debug)]
struct SaveV3 {
    previous: SaveV2,
    render_quality: SavedRenderQuality,
}

#[derive(Serialize, Deserialize, Debug)]
enum SavedRenderQuality {
    Mobile,
    Standard,
    High,
}

impl From<&PlayerProgress> for SaveV3 {
    fn from(progress: &PlayerProgress) -> Self {
        Self {
            previous: SaveV2::from(progress),
            render_quality: match progress.render_quality() {
                RenderQuality::Mobile => SavedRenderQuality::Mobile,
                RenderQuality::Standard => SavedRenderQuality::Standard,
                RenderQuality::High => SavedRenderQuality::High,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum SavedLanguage {
    Japanese,
    English,
}

impl From<&PlayerProgress> for SaveV2 {
    fn from(progress: &PlayerProgress) -> Self {
        Self {
            progress: SaveV1::from(progress),
            language: match progress.language() {
                Language::Japanese => SavedLanguage::Japanese,
                Language::English => SavedLanguage::English,
            },
            clears: progress
                .clears()
                .map(|(id, d)| (id.as_str().to_owned(), d.level()))
                .collect(),
        }
    }
}

impl From<&PlayerProgress> for SaveV1 {
    fn from(progress: &PlayerProgress) -> Self {
        Self {
            last_character: progress
                .last_character()
                .map(|c| c.as_str().to_owned()),
            games_played: progress.games_played(),
            records: progress
                .records()
                .map(|(stage, r)| StageRecordV1 {
                    stage: stage.as_str().to_owned(),
                    plays: r.plays(),
                    best_defeated: r.best_defeated(),
                })
                .collect(),
        }
    }
}

impl SaveV1 {
    /// Domainの検証付き生成を通す（読込時の値の検証）。
    fn into_domain(self) -> Result<PlayerProgress, ProgressError> {
        PlayerProgress::restore(
            self.last_character.map(PusherCharacterId::new),
            self.games_played,
            self.records
                .into_iter()
                .map(|r| (StageId::new(r.stage), r.plays, r.best_defeated)),
        )
    }
}

// ── エラー ────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SaveFileError {
    #[error("i/o error on {}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("not a NUDRA save file")]
    BadMagic,
    #[error("save file is truncated")]
    Truncated,
    #[error("save data length mismatch (header {expected}, actual {actual})")]
    LengthMismatch { expected: usize, actual: usize },
    #[error("save data is too large ({0} bytes)")]
    TooLarge(usize),
    #[error("save data checksum mismatch")]
    Checksum,
    #[error("save version {0} is newer than this game supports ({CURRENT_VERSION})")]
    NewerVersion(u16),
    #[error("unsupported save version {0}")]
    UnsupportedVersion(u16),
    #[error("failed to decode save data")]
    Decode(#[source] postcard::Error),
    #[error("failed to encode save data")]
    Encode(#[source] postcard::Error),
    #[error("save data is invalid")]
    Invalid(#[source] ProgressError),
    #[error("save data contains an invalid difficulty")]
    InvalidDifficulty(#[source] domain::difficulty::UnknownDifficulty),
    #[error("save file was written by a newer version; refusing to overwrite it")]
    ReadOnly,
    #[error("both the save file and its backup are unreadable")]
    Unrecoverable {
        #[source]
        main: Box<SaveFileError>,
        backup: Box<SaveFileError>,
    },
}

impl SaveFileError {
    /// ファイルの中身が壊れている（隔離してバックアップから復旧すべき）か。
    fn is_corrupt(&self) -> bool {
        matches!(
            self,
            Self::BadMagic
                | Self::Truncated
                | Self::LengthMismatch { .. }
                | Self::TooLarge(_)
                | Self::Checksum
                | Self::UnsupportedVersion(_)
                | Self::Decode(_)
                | Self::Invalid(_)
                | Self::InvalidDifficulty(_)
        )
    }
}

// ── 符号化 ────────────────────────────────────────────────

pub fn encode(progress: &PlayerProgress) -> Result<Vec<u8>, SaveFileError> {
    let payload = postcard::to_stdvec(&SaveV3::from(progress)).map_err(SaveFileError::Encode)?;
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|_| payload.len() <= MAX_PAYLOAD)
        .ok_or(SaveFileError::TooLarge(payload.len()))?;
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&CURRENT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

/// 読込時の検証: マジック → 版 → 長さ → CRC32 → デコード → Domainの検証。
/// 版の判定を長さ・CRCより先に行う（未来の版はヘッダーの形が変わり得るため）。
pub fn decode(bytes: &[u8]) -> Result<PlayerProgress, SaveFileError> {
    let (header, payload) = bytes
        .split_at_checked(HEADER_LEN)
        .ok_or(SaveFileError::Truncated)?;
    let field = |range: std::ops::Range<usize>| &header[range];
    if field(0..8) != MAGIC {
        return Err(SaveFileError::BadMagic);
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    if version > CURRENT_VERSION {
        return Err(SaveFileError::NewerVersion(version));
    }
    let length = u32::from_le_bytes([header[10], header[11], header[12], header[13]]) as usize;
    if length > MAX_PAYLOAD {
        return Err(SaveFileError::TooLarge(length));
    }
    if payload.len() != length {
        return Err(SaveFileError::LengthMismatch { expected: length, actual: payload.len() });
    }
    let checksum = u32::from_le_bytes([header[14], header[15], header[16], header[17]]);
    if crc32fast::hash(payload) != checksum {
        return Err(SaveFileError::Checksum);
    }
    // 新しい版を足すときは、古い版を最新の形へ移行する分岐をここに足す
    match version {
        1 => postcard::from_bytes::<SaveV1>(payload)
            .map_err(SaveFileError::Decode)?
            .into_domain()
            .map_err(SaveFileError::Invalid),
        2 | 3 => {
            let (save, quality) = if version == 2 {
                (
                    postcard::from_bytes::<SaveV2>(payload).map_err(SaveFileError::Decode)?,
                    RenderQuality::default(),
                )
            } else {
                let save: SaveV3 = postcard::from_bytes(payload).map_err(SaveFileError::Decode)?;
                let quality = match save.render_quality {
                    SavedRenderQuality::Mobile => RenderQuality::Mobile,
                    SavedRenderQuality::Standard => RenderQuality::Standard,
                    SavedRenderQuality::High => RenderQuality::High,
                };
                (save.previous, quality)
            };
            let language = match save.language {
                SavedLanguage::Japanese => Language::Japanese,
                SavedLanguage::English => Language::English,
            };
            let clears = save
                .clears
                .into_iter()
                .map(|(id, d)| {
                    Ok((
                        StageId::new(id),
                        Difficulty::new(d).map_err(SaveFileError::InvalidDifficulty)?,
                    ))
                })
                .collect::<Result<Vec<_>, SaveFileError>>()?;
            let mut progress = save
                .progress
                .into_domain()
                .map_err(SaveFileError::Invalid)?
                .restore_preferences(language, clears)
                .map_err(SaveFileError::Invalid)?;
            progress.set_render_quality(quality);
            Ok(progress)
        },
        other => Err(SaveFileError::UnsupportedVersion(other)),
    }
}

// ── Repository実装 ───────────────────────────────────────

/// 書き込みタスクへ渡せるよう `Clone`。読み取り専用の判定はすべての複製で共有する。
#[derive(Resource, Debug, Clone)]
pub struct FileProgressRepository {
    path: PathBuf,
    /// 未来の版のファイルを読んだら立てる。以後の保存を拒否する
    read_only: Arc<AtomicBool>,
}

impl FileProgressRepository {
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        Self {
            path: dir.as_ref().join(FILE_NAME),
            read_only: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 既定の保存先: 環境変数 `NUDRA_SAVE_DIR` → OSのユーザーデータ領域/nudra → `./save`。
    pub fn default_dir() -> PathBuf {
        std::env::var_os("NUDRA_SAVE_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|d| d.join("nudra")))
            .unwrap_or_else(|| PathBuf::from("save"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn sibling(&self, extension: &str) -> PathBuf {
        self.path.with_extension(extension)
    }
}

impl ProgressRepository for FileProgressRepository {
    type Error = SaveFileError;

    fn load(&self) -> Result<Option<PlayerProgress>, SaveFileError> {
        let backup = self.sibling("bak");
        let result = match read(&self.path) {
            Ok(Some(progress)) => return Ok(Some(progress)),
            // 退避と置き換えの間で中断された場合は、現行ファイルが無くバックアップだけが残る
            Ok(None) => read(&backup),
            Err(main) if main.is_corrupt() => {
                let corrupt = self.sibling("corrupt");
                if let Err(e) = fs::rename(&self.path, &corrupt) {
                    warn!(error = %e, path = %corrupt.display(), "failed to quarantine save file");
                }
                match read(&backup) {
                    Ok(Some(progress)) => {
                        warn!(error = %main, "save file was unreadable; restored from backup");
                        Ok(Some(progress))
                    },
                    Ok(None) => Err(main),
                    Err(backup) => Err(SaveFileError::Unrecoverable {
                        main: Box::new(main),
                        backup: Box::new(backup),
                    }),
                }
            },
            Err(other) => Err(other),
        };
        if matches!(result, Err(SaveFileError::NewerVersion(_))) {
            self.read_only.store(true, Ordering::Relaxed);
        }
        result
    }

    fn save(&mut self, progress: &PlayerProgress) -> Result<(), SaveFileError> {
        if self.read_only.load(Ordering::Relaxed) {
            return Err(SaveFileError::ReadOnly);
        }
        let bytes = encode(progress)?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(io_error(dir))?;
        }
        let temp = self.sibling("tmp");
        write_synced(&temp, &bytes)?;
        if self.path.exists() {
            let backup = self.sibling("bak");
            fs::rename(&self.path, &backup).map_err(io_error(&backup))?;
        }
        fs::rename(&temp, &self.path).map_err(io_error(&self.path))
    }
}

/// ファイルが無ければ `Ok(None)`。巨大なファイルは読み込む前に拒否する。
fn read(path: &Path) -> Result<Option<PlayerProgress>, SaveFileError> {
    let size = match fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(path)(e)),
    };
    if size > (HEADER_LEN + MAX_PAYLOAD) as u64 {
        return Err(SaveFileError::TooLarge(usize::try_from(size).unwrap_or(usize::MAX)));
    }
    let bytes = fs::read(path).map_err(io_error(path))?;
    decode(&bytes).map(Some)
}

/// 書き込んだ内容をディスクへ同期してから閉じる（電源断で中身の無いファイルを残さない）。
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), SaveFileError> {
    let mut file = fs::File::create(path).map_err(io_error(path))?;
    file.write_all(bytes).map_err(io_error(path))?;
    file.sync_all().map_err(io_error(path))
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> SaveFileError + '_ {
    move |source| SaveFileError::Io { path: path.to_owned(), source }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

    fn progress(results: &[(&str, u32)]) -> PlayerProgress {
        let mut progress = PlayerProgress::default();
        progress.select_character(PusherCharacterId::new("mop_rabbit"));
        for (stage, defeated) in results {
            progress
                .record_session_result(&StageId::new(*stage), *defeated, Difficulty::EASY, false)
                .expect("valid result");
        }
        progress
    }

    /// 任意の版番号・本体で、ヘッダー（長さ・CRC）だけ正しいファイルを作る。
    fn frame(version: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(payload.len())
                .expect("small")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    // ── 符号化・読込時の検証 ──

    #[rstest]
    fn encoded_progress_decodes_to_the_same_value() {
        let original = progress(&[("stage_01", 4), ("stage_02", 9), ("stage_01", 7)]);
        assert_eq!(decode(&encode(&original).expect("encode")).expect("decode"), original);
    }

    #[test]
    fn version_2_keeps_language_and_stage_clear_unlocks() {
        let mut original = PlayerProgress::default();
        original.set_language(Language::English);
        original
            .record_session_result(&StageId::new("stage_01"), 8, Difficulty::EASY, true)
            .expect("won");
        let bytes = frame(2, &postcard::to_stdvec(&SaveV2::from(&original)).expect("encode"));
        assert_eq!(&bytes[8..10], &2u16.to_le_bytes());
        let restored = decode(&bytes).expect("decode");
        assert_eq!(restored, original);
        assert_eq!(
            restored
                .unlocked_difficulty(&StageId::new("stage_01"))
                .level(),
            2
        );
    }

    #[test]
    fn version_2_rejects_invalid_clear_difficulties() {
        let save = SaveV2 {
            progress: SaveV1::from(&progress(&[("s", 1)])),
            language: SavedLanguage::Japanese,
            clears: vec![("s".into(), 5)],
        };
        let bytes = frame(2, &postcard::to_stdvec(&save).expect("encode"));
        assert!(matches!(decode(&bytes), Err(SaveFileError::InvalidDifficulty(_))));
    }

    #[rstest]
    #[case(RenderQuality::Mobile)]
    #[case(RenderQuality::Standard)]
    #[case(RenderQuality::High)]
    fn version_3_round_trips_quality_and_existing_progress(#[case] quality: RenderQuality) {
        let mut original = progress(&[("stage_01", 9)]);
        original.set_language(Language::English);
        original.set_render_quality(quality);
        let bytes = encode(&original).expect("encode");
        assert_eq!(&bytes[8..10], &3u16.to_le_bytes());
        assert_eq!(decode(&bytes).expect("decode"), original);
    }

    #[test]
    fn version_2_golden_payload_migrates_to_default_quality() {
        // Frozen postcard layout: no character, no games, no records, English, no clears.
        const GOLDEN_V2_PAYLOAD: &[u8] = &[0, 0, 0, 1, 0];
        let restored = decode(&frame(2, GOLDEN_V2_PAYLOAD)).expect("golden v2");
        assert_eq!(restored.language(), Language::English);
        assert_eq!(restored.render_quality(), RenderQuality::default());
        let migrated = encode(&restored).expect("migrate to v3");
        assert_eq!(decode(&migrated).expect("decode v3"), restored);
    }

    #[test]
    fn version_3_rejects_an_unknown_quality() {
        assert!(matches!(
            decode(&frame(3, &[0, 0, 0, 0, 0, 3])),
            Err(SaveFileError::Decode(_))
        ));
    }

    /// 版1のファイル形式を固定する。この値が変わるなら、版2を追加して移行すること。
    #[rstest]
    fn version_1_golden_file_still_decodes() {
        const GOLDEN_V1: &[u8] = &[
            b'N', b'U', b'D', b'R', b'A', b'S', b'A', b'V', // magic
            1, 0, // version 1
            25, 0, 0, 0, // payload length
            0x9E, 0xCB, 0xC4, 0xE2, // crc32
            1, 10, b'm', b'o', b'p', b'_', b'r', b'a', b'b', b'b', b'i',
            b't', // Some("mop_rabbit")
            3,    // games_played
            1,    // 1 record
            8, b's', b't', b'a', b'g', b'e', b'_', b'0', b'1', // "stage_01"
            3, 7, // plays, best_defeated
        ];
        assert_eq!(
            decode(GOLDEN_V1).expect("golden v1"),
            progress(&[("stage_01", 7), ("stage_01", 2), ("stage_01", 0)])
        );
    }

    #[rstest]
    #[case::empty(vec![], "Truncated")]
    #[case::bad_magic(frame(1, b"x").into_iter().enumerate().map(|(i, b)| if i == 0 { b'X' } else { b }).collect(), "BadMagic")]
    #[case::newer_version(frame(CURRENT_VERSION + 1, b"anything"), "NewerVersion")]
    #[case::unknown_old_version(frame(0, b"anything"), "UnsupportedVersion")]
    #[case::too_large({ let mut f = frame(1, b""); f[10..14].copy_from_slice(&u32::MAX.to_le_bytes()); f }, "TooLarge")]
    #[case::payload_cut(frame(1, b"abcdef")[..HEADER_LEN + 3].to_vec(), "LengthMismatch")]
    #[case::trailing_garbage({ let mut f = frame(1, b"abc"); f.push(0); f }, "LengthMismatch")]
    #[case::bit_flip({ let mut f = encode(&progress(&[("s", 1)])).expect("encode"); *f.last_mut().expect("payload") ^= 1; f }, "Checksum")]
    #[case::garbage_payload(frame(1, &[0xFF, 0xFF, 0xFF]), "Decode")]
    #[case::invalid_values(
        frame(1, &postcard::to_stdvec(&SaveV1 {
            last_character: None,
            games_played: 1,
            records: vec![StageRecordV1 { stage: "s".into(), plays: 0, best_defeated: 0 }],
        }).expect("encode")),
        "Invalid"
    )]
    fn broken_files_are_rejected(#[case] bytes: Vec<u8>, #[case] expected: &str) {
        let err = decode(&bytes).expect_err("must be rejected");
        assert!(format!("{err:?}").starts_with(expected), "{err:?}");
    }

    proptest! {
        #[test]
        fn any_progress_round_trips(results in prop::collection::vec((0u8..5, any::<u32>()), 0..30)) {
            let names: Vec<String> = results.iter().map(|(s, _)| format!("stage_{s}")).collect();
            let results: Vec<(&str, u32)> =
                names.iter().map(String::as_str).zip(results.iter().map(|(_, d)| *d)).collect();
            let original = progress(&results);
            prop_assert_eq!(decode(&encode(&original).expect("encode")).expect("decode"), original);
        }

        /// どんなバイト列を読ませても、パニックせずに検証済みの値かエラーを返す。
        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
            let _ = decode(&bytes);
        }
    }

    // ── 保存・バックアップ・復旧 ──

    fn repo() -> (TempDir, FileProgressRepository) {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = FileProgressRepository::in_dir(dir.path());
        (dir, repo)
    }

    #[rstest]
    fn load_without_any_file_is_none() {
        let (_dir, repo) = repo();
        assert_eq!(repo.load().expect("load"), None);
    }

    #[rstest]
    fn saved_progress_loads_back_and_creates_the_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut repo = FileProgressRepository::in_dir(dir.path().join("nested"));
        let p = progress(&[("stage_01", 3)]);
        repo.save(&p).expect("save");
        assert_eq!(repo.load().expect("load"), Some(p));
        assert!(!repo.sibling("tmp").exists(), "temporary file is renamed away");
    }

    #[rstest]
    fn previous_save_is_kept_as_backup() {
        let (_dir, mut repo) = repo();
        let (first, second) = (progress(&[("s", 1)]), progress(&[("s", 1), ("s", 2)]));
        repo.save(&first).expect("save");
        repo.save(&second).expect("save");
        let backup = fs::read(repo.sibling("bak")).expect("backup exists");
        assert_eq!(decode(&backup).expect("backup decodes"), first);
        assert_eq!(repo.load().expect("load"), Some(second));
    }

    #[rstest]
    fn corrupt_save_is_quarantined_and_restored_from_backup() {
        let (_dir, mut repo) = repo();
        let first = progress(&[("s", 1)]);
        repo.save(&first).expect("save");
        repo.save(&progress(&[("s", 1), ("s", 2)]))
            .expect("save");
        fs::write(repo.path(), b"garbage").expect("corrupt");

        assert_eq!(repo.load().expect("recovered"), Some(first));
        assert_eq!(fs::read(repo.sibling("corrupt")).expect("quarantined"), b"garbage");
        assert!(!repo.path().exists());
    }

    #[rstest]
    fn missing_save_after_interrupted_rotation_uses_backup() {
        let (_dir, mut repo) = repo();
        let first = progress(&[("s", 1)]);
        repo.save(&first).expect("save");
        repo.save(&progress(&[("s", 1), ("s", 2)]))
            .expect("save");
        fs::remove_file(repo.path()).expect("simulate crash between renames");
        assert_eq!(repo.load().expect("load"), Some(first));
    }

    #[rstest]
    fn corrupt_save_without_backup_is_an_error() {
        let (_dir, repo) = repo();
        fs::write(repo.path(), b"garbage").expect("corrupt");
        assert!(matches!(repo.load(), Err(SaveFileError::Truncated)));
    }

    #[rstest]
    fn corrupt_save_and_backup_are_unrecoverable() {
        let (_dir, repo) = repo();
        fs::write(repo.path(), b"garbage").expect("corrupt");
        fs::write(repo.sibling("bak"), b"garbage too").expect("corrupt");
        assert!(matches!(repo.load(), Err(SaveFileError::Unrecoverable { .. })));
    }

    #[rstest]
    fn newer_version_file_is_never_overwritten() {
        let (_dir, mut repo) = repo();
        let newer = frame(CURRENT_VERSION + 1, b"from the future");
        fs::write(repo.path(), &newer).expect("write");

        assert!(matches!(repo.load(), Err(SaveFileError::NewerVersion(_))));
        assert!(matches!(repo.save(&progress(&[])), Err(SaveFileError::ReadOnly)));
        assert_eq!(fs::read(repo.path()).expect("still there"), newer);
    }
}
