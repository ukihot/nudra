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

## エラーの鉄則

- domain: 集約・操作ごとの thiserror enum。巨大な `DomainError` は作らない。テストのため `PartialEq` を付ける。
- application: ユースケースごとの thiserror enum。domainのErrorは `#[error(transparent)]` + `#[from]` で包む。
- game: 文脈付けは `anyhow::Context`。`?` で `BevyError` へ。回復動作が要る失敗は `pipe` で受ける。
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

game crateを触ったときは `-p game` も加える（Bevyのビルドが入るため時間がかかる）。

## スタイル

- Rust edition 2024 のモダンな書き方（let-chains、`impl Trait` 引数、`#[must_use]` など）を使う。
- フォーマットは nightly rustfmt（`rustfmt.toml`）に従う。手で整形しない。
- ドキュメントコメントは日本語、Errorメッセージとログは英語。
- 依存crateのバージョンは `[workspace.dependencies]` で一元管理する。glamは bevy_math と同じバージョン。

## 環境メモ（Windows）

- `python3` は Microsoft Store のエイリアスで動かない。スクリプトはbashかRustで書く。
- Bevy・Avianのソースは `~/.cargo/registry/src/*/bevy_*-0.20.*` と `avian3d-0.8.*` にある。APIは推測せずここで確認する。
- gitのコミットは頼まれたときだけ行う。
