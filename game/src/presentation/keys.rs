//! Presentation で使う文字列定数の集約場所（このファイル以外に文字列リテラルを書かない）。
//!
//! 表示文はここにも書かない。ロケールのカタログ（`game/locales/<言語>/ui.json`）に書く。
//! ここに置くのは、翻訳キー・引数名・言語コード・アセットのパスだけ。
//! キーは enum で表し、Rust側の打ち間違いをコンパイル時に防ぐ。カタログとの整合は
//! `game/tests/presentation_text.rs` で検査する。アセットのパスが増えたら別ファイルへ分ける。

use bevy_intl::{I18nMode, I18nText};

/// 【仮決定】日本語グリフを持つフォントを同梱するまでは英語を既定にする（Bevyの既定フォントは日本語を含まない）。
pub const DEFAULT_LANG: &str = "en";
pub const FALLBACK_LANG: &str = "en";
const UI_FILE: &str = "ui";

pub const VISUAL_MANIFEST: &str = "visuals.visuals.ron";
pub const VISUAL_MANIFEST_EXTENSIONS: &[&str] = &["visuals.ron"];

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
}

impl Ui {
    pub const ALL: [Self; 11] = [
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
}

impl Arg {
    pub const ALL: [Self; 8] = [
        Self::Index,
        Self::Name,
        Self::Wave,
        Self::Enemies,
        Self::Defeated,
        Self::Level,
        Self::Best,
        Self::Time,
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
        }
    }
}
