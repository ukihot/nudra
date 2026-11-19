//! Presentation層の文字列の規約（CLAUDE.md）。
//! - 表示文はロケールのカタログ（game/locales/<言語>/ui.json）に書き、コードに書かない。
//! - キー・引数名・アセットのパスは presentation/keys.rs だけに置く。

use std::{collections::BTreeSet, fs, path::Path};

use game::presentation::keys::{Arg, Ui};

const SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
const LOCALES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/locales");
/// 文字列を置いてよい唯一のファイル
const ALLOWED: &str = "keys.rs";

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
            let source = fs::read_to_string(path).expect("readable");
            source
                .lines()
                .enumerate()
                // コメント（`//` 以降）は対象外
                .filter(|(_, line)| line.split("//").next().is_some_and(|code| code.contains('"')))
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

fn text<'a>(catalog: &'a serde_json::Map<String, serde_json::Value>, ui: Ui) -> &'a str {
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
