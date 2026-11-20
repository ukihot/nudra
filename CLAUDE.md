# CLAUDE.md

このリポジトリで作業するときの鉄則。迷ったら [docs/basic-design.md](docs/basic-design.md) が正。

## 作業の前に

- 要件は [docs/requirements.md](docs/requirements.md)、設計は [docs/basic-design.md](docs/basic-design.md)。実装はこの2つに従う。
- 設計と違う実装にした場合は、同じ作業の中で basic-design.md を更新する。
- 【仮決定】【要検証】の項目に触れたら、設計書の12章（仮決定・未決事項一覧）も確認する。

## アーキテクチャの鉄則

- `crates/domain` は Bevy・Avian・anyhow・tracing に依存しない。依存は glam・thiserror のみ。
- `mod.rs` は使わない。サブモジュールは `foo.rs` + `foo/` ディレクトリで置く。
- Entity / ValueObject / AggregateRoot の共通traitは作らない。traitはRepositoryと、本当に差し替えが必要な境界だけ。
- 集約のメソッドは**全か無か**。検証を先に済ませ、`Err` を返すときは状態を一切変更しない。
- 不正な物理パラメータはDomainで生成できないようにする。厳密に正の有限値を要する値（質量・半径・寸法・強化倍率など）は
  `Positive`。0が有効な値（速度・摩擦・反発係数など）には使わない。積は `checked_mul`。
- メダル・敵・Pusherの位置や速度はECS（Avian）が正本。Domainに二重に持たない。
- 外側への指示（射出・出現など）はユースケースの戻り値で返す。出力ポートtraitは作らない。
- game crate の依存性逆転は Bevy 流（基本設計 5.6）。Infrastructure と Presentation は互いを参照せず、
  `game/src/port`（`request.rs`＝入力、`response.rs`＝出力、`appearance.rs`＝見た目の指定）の
  Message・読み取り専用Resource・Component だけで連携する。ユースケースを呼ぶのはInfrastructureだけ。
- ゲームのルール（可否の判定）を game 側に書かない。ユースケースの `Result` を `RequestStatus` に写すだけにする。
- 見た目は Infrastructure が `Appearance`（key + 仮形状）で指定し、Presentation が `visuals.visuals.ron` で解決する。
  アセットの差し替えはこの定義ファイルの書き換えで行い、コードを変えない。

## オフラインファーストの鉄則

- ゲームプレイはネットワーク・ユーザー認証・外部サーバーに依存しない。ローカルのセーブファイルが唯一の正本（基本設計 5.7）。
- セーブは serde + postcard のバイナリ。ヘッダー（マジック・版・長さ・CRC32）付き。
- 保存形式（`SaveV1` …）はリリース後に変更しない。変更するときは新しい版の構造体と移行処理を足し、
  版1のゴールデンテスト（固定バイト列）は消さない。未来の版のファイルは読み取り専用にして上書きしない。
- 保存は「一時ファイル → 同期 → 現行を `.bak` へ → 置き換え」。読めないファイルは `.corrupt` へ隔離して `.bak` から復旧。
- 読込時は必ず Domain の検証付き生成（`PlayerProgress::restore` など）を通す。
- 保存・読込の失敗でゲームを止めない（記録はメモリに保持し、警告ログ）。
- テストは実ユーザーのデータ領域へ書かない。`FileProgressRepository::in_dir(一時ディレクトリ)` を差し込む。

## Presentation の文字列の鉄則

- Presentation 層（`game/src/presentation*`）に文字列リテラルを書かない。`game/tests/presentation_text.rs` が検査する。
- 表示文は `game/locales/<言語>/ui.json`（bevy-intl のカタログ）に書き、`I18nText` で表示する。
- 翻訳キー（`Ui`）・引数名（`Arg`）・言語コード・アセットのパスは `presentation/keys.rs` だけに置く。
- 表示文を足すときは: `Ui` にバリアントを足す → `Ui::ALL` に足す → **全言語**のカタログに同じキーと同じ `{{引数}}` を足す。
- カタログは `bundle-only` でビルド時に埋め込まれる（場所は `.cargo/config.toml` の `BEVY_INTL_MESSAGES_DIR`）。

## エラーの鉄則

- domain: 集約・操作ごとの thiserror enum。巨大な `DomainError` は作らない。テストのため `PartialEq` を付ける。
- application: ユースケースごとの thiserror enum。domainのErrorは `#[error(transparent)]` + `#[from]` で包む。
- game: 文脈付けは `anyhow::Context`。`?` で `BevyError` へ。回復動作が要る失敗は `pipe` で受ける。
- Bevy 0.20 の prelude は `ContextExt`（`.context()`）を含み `anyhow::Context` と衝突する。
  anyhow で文脈を付けるファイルでは `bevy::prelude::*` を glob で読み込まず、個別に import する。
- 「操作の拒否」（クールダウン中の射出など）は型付きErrorだが、`?` でエラーハンドラへ流さず `match` する。
- 物理由来の結果（落下・到達）はエラーにしない。遅れて届いた結果は無視する。
- `unwrap()` 禁止。`expect("理由")` は不変条件上起こり得ない場合だけ。
- 下位Errorを `source` で保持するとき、上位のメッセージに下位のメッセージを埋め込まない。

## TDDの鉄則

1. **RED**: 先にテストを書き、実装は `todo!()` のスタブにして、テストが**正しい理由で落ちる**ことを実際に実行して確認する。
   コンパイルエラーはREDに数えない。
2. **GREEN**: テストを通す最小の実装を書く。
3. **REFACTOR**: パフォーマンス（毎tick呼ばれる箇所の不要な確保など）・可読性・処理の短さを改善し、再度GREENを確認する。
4. **正常系と異常系を両方**単体テストにする。境界値（0、負数、NaN、±∞、オーバーフロー）を忘れない。
5. 仕上げに、主要ロジックを1か所ずつわざと壊して（例: 条件を `false` に、代入を削除）、テストが落ちることを確認する。
   落ちない変更があれば、テストが足りない。

テストの道具:

| 道具 | 使い方 |
|---|---|
| rstest | `#[fixture]` で共通データ、`#[case::name(...)]` で正常系・異常系の表形式テスト |
| proptest | 不変条件の性質テスト（範囲を出ない、高々1回、全か無か など） |
| mockall | 別crateのtraitは `mockall::mock!` でモック化し、呼び出し引数・回数まで検証する |
| cargo-nextest | テストの実行はすべてこれで行う |

- `proptest-regressions/` は削除せずコミットする（過去に見つかった失敗例の再検証用）。
- テスト用ヘルパー: domainは `crate::positive::fixture::p`、applicationは `crate::test_support::p`。

## コード編集後の必須手順

コードを編集したら、作業の最後に**必ずこの順で**実行し、すべて通してから報告する。

```sh
cargo +nightly fmt --all
cargo clippy -p domain -p application --all-targets    # 警告ゼロ
cargo nextest run -p domain -p application             # 全件GREEN
```

game crateを触ったときは `-p game` も加える。Bevyのビルドで数分かかるため、ツールのタイムアウトに注意してバックグラウンドで実行する。
glam のバージョンは bevy_math と同じにする（`cargo tree -i glam` で1つだけであることを確認）。ずれると `Vec3` が別の型になる。

## スタイル

- Rust edition 2024 のモダンな書き方（let-chains、`impl Trait` 引数、`#[must_use]` など）を使う。
- フォーマットは nightly rustfmt（`rustfmt.toml`）に従う。手で整形しない。
- ドキュメントコメントは日本語、Errorメッセージとログは英語。
- 依存crateのバージョンは `[workspace.dependencies]` で一元管理する。glamは bevy_math と同じバージョン。

## 環境メモ（Windows）

- `python3` は Microsoft Store のエイリアスで動かない。スクリプトはbashかRustで書く。
- Bevy・Avianのソースは `~/.cargo/registry/src/*/bevy_*-0.20.*` と `avian3d-0.8.*` にある。APIは推測せずここで確認する。
- gitのコミットは頼まれたときだけ行う。
