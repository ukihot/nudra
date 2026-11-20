//! Presentation で使う文字列定数の集約場所（このファイル以外に文字列リテラルを書かない）。
//!
//! 表示文はここにも書かない。ロケールのカタログ（`game/locales/<言語>/ui.json`）に書く。
//! ここに置くのは、翻訳キー・引数名・言語コード・アセットのパスだけ。
//! キーは enum で表し、Rust側の打ち間違いをコンパイル時に防ぐ。カタログとの整合は
//! `game/tests/presentation_text.rs` で検査する。アセットのパスが増えたら別ファイルへ分ける。

use bevy_intl::{I18nMode, I18nText};

/// 初回は日本語。以後は保存された言語を使う。
pub const DEFAULT_LANG: &str = "ja";
pub const FALLBACK_LANG: &str = "en";
pub const JAPANESE_LANG: &str = "ja";
pub const ENGLISH_LANG: &str = "en";
pub const UI_FONT: &str = "fonts/NotoSansJP.ttf";
const UI_FILE: &str = "ui";

pub const VISUAL_MANIFEST: &str = "visuals.visuals.ron";
pub const VISUAL_MANIFEST_EXTENSIONS: &[&str] = &["visuals.ron"];
pub const RENDER_PROFILE: &str = "render.profiles.ron";
pub const RENDER_PROFILE_SOURCE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/render.profiles.ron");
pub const RENDER_PROFILE_EXTENSIONS: &[&str] = &["profiles.ron"];
pub const MEDAL_ICON: &str = "ui/medal_normal.png";
pub const PUSHER_PORTRAIT: &str = "ui/ao_umiushi.png";
pub const BUNDLED_RENDER_PROFILE: &str = include_str!("../../assets/render.profiles.ron");
pub const VALID_RENDER_PROFILE: &str = "bundled render profiles are valid";
pub const INVALID_RENDER_PROFILE: &str = "invalid render profiles; keeping the last valid look";

/// UIの表示文のキー。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ui {
    Title,
    ChoosePusher,
    CharacterEntry,
    Hud,
    Controls,
    GameOver,
    Defeated,
    BestRecord,
    NewRecord,
    PlayTime,
    Continue,
    Victory,
    Evolve,
    LevelUp,
    Fuse,
    NewMedal,
    Prepare,
    Options,
    Credits,
    Back,
    Language,
    Japanese,
    English,
    CreditsBody,
    ChooseStage,
    Difficulty,
    DifficultyLevel,
    LockedDifficulty,
    Start,
    PreparationControls,
    Expedition,
    HudWave,
    HudGrowth,
    Loadout,
    RenderQuality,
    QualityMobile,
    QualityStandard,
    QualityHigh,
}

impl Ui {
    pub const ALL: [Self; 38] = [
        Self::Title,
        Self::ChoosePusher,
        Self::CharacterEntry,
        Self::Hud,
        Self::Controls,
        Self::GameOver,
        Self::Defeated,
        Self::BestRecord,
        Self::NewRecord,
        Self::PlayTime,
        Self::Continue,
        Self::Victory,
        Self::Evolve,
        Self::LevelUp,
        Self::Fuse,
        Self::NewMedal,
        Self::Prepare,
        Self::Options,
        Self::Credits,
        Self::Back,
        Self::Language,
        Self::Japanese,
        Self::English,
        Self::CreditsBody,
        Self::ChooseStage,
        Self::Difficulty,
        Self::DifficultyLevel,
        Self::LockedDifficulty,
        Self::Start,
        Self::PreparationControls,
        Self::Expedition,
        Self::HudWave,
        Self::HudGrowth,
        Self::Loadout,
        Self::RenderQuality,
        Self::QualityMobile,
        Self::QualityStandard,
        Self::QualityHigh,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::ChoosePusher => "choose_pusher",
            Self::CharacterEntry => "character_entry",
            Self::Hud => "hud",
            Self::Controls => "controls",
            Self::GameOver => "game_over",
            Self::Defeated => "defeated",
            Self::BestRecord => "best_record",
            Self::NewRecord => "new_record",
            Self::PlayTime => "play_time",
            Self::Continue => "continue",
            Self::Victory => "victory",
            Self::Evolve => "evolve",
            Self::LevelUp => "level_up",
            Self::Fuse => "fuse",
            Self::NewMedal => "new_medal",
            Self::Prepare => "prepare",
            Self::Options => "options",
            Self::Credits => "credits",
            Self::Back => "back",
            Self::Language => "language",
            Self::Japanese => "japanese",
            Self::English => "english",
            Self::CreditsBody => "credits_body",
            Self::ChooseStage => "choose_stage",
            Self::Difficulty => "difficulty",
            Self::DifficultyLevel => "difficulty_level",
            Self::LockedDifficulty => "locked_difficulty",
            Self::Start => "start",
            Self::PreparationControls => "preparation_controls",
            Self::Expedition => "expedition",
            Self::HudWave => "hud_wave",
            Self::HudGrowth => "hud_growth",
            Self::Loadout => "loadout",
            Self::RenderQuality => "render_quality",
            Self::QualityMobile => "quality_mobile",
            Self::QualityStandard => "quality_standard",
            Self::QualityHigh => "quality_high",
        }
    }

    pub fn text(self) -> I18nText {
        I18nText::new(UI_FILE, self.key())
    }

    /// `{{引数名}}` を値で置き換える表示文。
    pub fn with(self, args: impl IntoIterator<Item = (Arg, String)>) -> I18nText {
        let args = args
            .into_iter()
            .map(|(arg, value)| (arg.key().to_owned(), value))
            .collect();
        I18nText { mode: I18nMode::Args(args), ..self.text() }
    }
}

/// 表示文の引数名（カタログでは `{{name}}` のように書く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    Index,
    Name,
    Wave,
    Enemies,
    Defeated,
    Level,
    Best,
    Time,
    Gauge,
}

impl Arg {
    pub const ALL: [Self; 9] = [
        Self::Index,
        Self::Name,
        Self::Wave,
        Self::Enemies,
        Self::Defeated,
        Self::Level,
        Self::Best,
        Self::Time,
        Self::Gauge,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Name => "name",
            Self::Wave => "wave",
            Self::Enemies => "enemies",
            Self::Defeated => "defeated",
            Self::Level => "level",
            Self::Best => "best",
            Self::Time => "time",
            Self::Gauge => "gauge",
        }
    }
}

/// メダルIDは翻訳カタログのキーと共通。
pub fn medal_name(medal: &domain::id::MedalCode) -> I18nText {
    I18nText::new("medals", medal.as_str())
}

pub fn stage_name(stage: &domain::id::StageId) -> I18nText {
    I18nText::new("stages", stage.as_str())
}
