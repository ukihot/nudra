//! Presentation層の文字列の規約（CLAUDE.md）。
//! - 表示文はロケールのカタログ（game/locales/<言語>/ui.json）に書き、コードに書かない。
//! - キー・引数名・アセットのパスは presentation/keys.rs だけに置く。
//! - 例外: presentation/play_time.rs だけは、数値の書式指定のリテラル（文言を含まない）を許可する。

use std::{collections::BTreeSet, fs, path::Path};

use game::presentation::keys::{Arg, Ui};

const SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
const LOCALES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/locales");
/// 文字列を置いてよい唯一のファイル
const ALLOWED: &str = "keys.rs";
/// 数値の書式指定のリテラルだけを許可するファイル
const FORMAT_ONLY: &str = "play_time.rs";

/// 行の中の文字列リテラルの中身（`"…"`）を取り出す。
fn literals(code: &str) -> Vec<&str> {
    code.split('"').skip(1).step_by(2).collect()
}

/// `{…}` の置き換え部分を除いて、英字も非ASCII（日本語など）も含まない＝文言ではなく数値の書式。
fn is_format_only(literal: &str) -> bool {
    let mut depth = 0usize;
    literal.chars().all(|c| {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {},
            c if c.is_alphabetic() || !c.is_ascii() => return false,
            _ => {},
        }
        true
    })
}

#[test]
fn format_only_rule_accepts_numbers_and_rejects_words() {
    assert!(is_format_only("{minutes:02}:{seconds:02}.{hundredths:02}"));
    assert!(is_format_only("00:01.01"));
    assert!(!is_format_only("Time {t}"));
    assert!(!is_format_only("プレイ時間"));
    assert!(!is_format_only("{a}x"));
}

#[test]
fn presentation_has_no_string_literals() {
    let src = Path::new(SRC);
    let mut files = vec![src.join("presentation.rs")];
    files.extend(
        fs::read_dir(src.join("presentation"))
            .expect("dir")
            .flatten()
            .map(|e| e.path()),
    );

    let violations: Vec<String> = files
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name != ALLOWED))
        .flat_map(|path| {
            let format_only = path
                .file_name()
                .is_some_and(|name| name == FORMAT_ONLY);
            let source = fs::read_to_string(path).expect("readable");
            source
                .lines()
                .enumerate()
                // コメント（`//` 以降）は対象外
                .filter(|(_, line)| {
                    let code = line.split("//").next().unwrap_or_default();
                    code.contains('"')
                        && !(format_only && literals(code).iter().all(|l| is_format_only(l)))
                })
                .map(|(no, line)| format!("{}:{}: {}", path.display(), no + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        violations.is_empty(),
        "string literals in presentation:\n{}",
        violations.join("\n")
    );
}

fn catalog(lang: &str) -> serde_json::Map<String, serde_json::Value> {
    let path = Path::new(LOCALES).join(lang).join("ui.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn languages() -> Vec<String> {
    let mut langs: Vec<String> = fs::read_dir(LOCALES)
        .expect("locales dir")
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    langs.sort();
    langs
}

#[test]
fn every_locale_has_exactly_the_keys_in_use() {
    let expected: BTreeSet<&str> = Ui::ALL.iter().map(|ui| ui.key()).collect();
    for lang in languages() {
        let keys: BTreeSet<String> = catalog(&lang).keys().cloned().collect();
        let keys: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
        assert_eq!(
            keys, expected,
            "locale {lang} must define exactly the keys in Ui::ALL"
        );
    }
    assert_eq!(languages(), ["en", "ja"]);
}

/// 表示文中の `{{名前}}` をすべて抜き出す。
fn placeholders(text: &str) -> BTreeSet<String> {
    text.split("{{")
        .skip(1)
        .filter_map(|rest| rest.split_once("}}"))
        .map(|(name, _)| name.to_owned())
        .collect()
}

fn text(catalog: &serde_json::Map<String, serde_json::Value>, ui: Ui) -> &str {
    catalog[ui.key()]
        .as_str()
        .unwrap_or_else(|| panic!("{} must be a string", ui.key()))
}

#[test]
fn placeholders_are_defined_args() {
    let defined: BTreeSet<String> = Arg::ALL.iter().map(|a| a.key().to_owned()).collect();
    for lang in languages() {
        let catalog = catalog(&lang);
        for ui in Ui::ALL {
            let unknown: Vec<_> = placeholders(text(&catalog, ui))
                .difference(&defined)
                .cloned()
                .collect();
            assert!(
                unknown.is_empty(),
                "{lang}/{}: undefined placeholders {unknown:?}",
                ui.key()
            );
        }
    }
}

#[test]
fn placeholders_match_between_locales() {
    let base = catalog("en");
    for lang in languages() {
        let other = catalog(&lang);
        for ui in Ui::ALL {
            assert_eq!(
                placeholders(text(&other, ui)),
                placeholders(text(&base, ui)),
                "{lang}/{}: placeholders differ from en",
                ui.key()
            );
        }
    }
}

#[test]
fn placeholder_extraction_finds_every_name() {
    assert_eq!(
        placeholders("{{wave}} / {{waves}}x{{"),
        BTreeSet::from(["wave".into(), "waves".into()])
    );
    assert!(placeholders("no args").is_empty());
}

#[test]
fn every_defined_medal_has_a_name_in_every_locale() {
    let specs: game::infrastructure::data::SpecsAsset =
        ron::from_str(include_str!("../assets/data/game.specs.ron")).expect("valid specs");
    // メダル名カタログは全定義IDと一致し、属性名をUI側に直書きしない。
    let catalog = specs.to_domain().expect("valid catalog");
    let en: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!("../locales/en/medals.json")).expect("en medals");
    let ja: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!("../locales/ja/medals.json")).expect("ja medals");
    assert_eq!(en.keys().collect::<BTreeSet<_>>(), ja.keys().collect::<BTreeSet<_>>());
    assert_eq!(
        en.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        catalog
            .medals()
            .map(|m| m.as_str())
            .collect::<BTreeSet<_>>()
    );
    assert!(
        en.values()
            .chain(ja.values())
            .all(|v| v.as_str().is_some_and(|s| !s.is_empty()))
    );
    assert_eq!(ja["sauna"], "サウナ");
    assert_eq!(ja["spicy_food"], "激辛");
    assert_eq!(ja["wish"], "願い");
    assert_eq!(ja["chainsaw"], "チェーンソー");
}
