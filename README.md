# NUDRA

物理演算型メダル落とし × タワーディフェンス。

プレイヤーは固定カタパルトからメダルを撃ち込み、レール上を自律往復する味方「Pusher」と一緒にメダルの山を押し崩して、
橋を渡って侵入してくる敵を奈落へ落とします。敵がGoalに到達したら敗北です。

> 「メダルを撃ち込み、味方が押し、敵を奈落へ落とす」

## 技術スタック

| 用途 | 採用 |
|---|---|
| 言語 | Rust（edition 2024） |
| エンジン | Bevy 0.20 |
| 物理 | Avian 0.8（avian3d） |
| エラー | thiserror 2 / anyhow 1 / tracing |
| テスト | cargo-nextest / rstest / proptest / mockall |
| 設計 | DDD + Clean Architecture + ECS、テスト駆動開発 |

## 構成

```
crates/
  domain/        ゲームの意味とルール。Bevy・Avianに依存しない純粋なRust
  application/   ユースケース（開始・射出・Wave・物理結果の判定・成長・進行記録）
game/            Bevy + Avian。Infrastructure と Presentation（実行バイナリ nudra）
  assets/        定義データ（data/*.ron）と見た目の定義（visuals.visuals.ron）
  locales/       表示文のカタログ（ja / en）
docs/
  requirements.md   要件定義書
  basic-design.md   基本設計書
```

依存の向きは `game → application → domain`（と `game → domain`）。domainは外側に依存しません。
game の中では Infrastructure と Presentation が互いを参照せず、`game/src/port`（入力 `request.rs`・出力 `response.rs`・見た目 `appearance.rs`）の Message・Resource だけで連携します。

## 開発状況

- [x] domain: 集約（Stage / GameSession / PusherCharacter / PlayerProgress）、定義データ、射出・経路・成長ルール
- [x] application: ユースケース一式
- [x] セーブ: オフラインファースト（ローカルのバイナリファイル、版管理・バックアップ・読込時検証）
- [x] game: 射出・敵の侵攻と撃破・4Waveと勝敗・進化中の時計／物理停止・進化画面・メダル選択
- [x] メダル定義: ノーマル1種・素材16種・錬成72種（全89種）、日英の表示名、[錬成表](docs/medal-fusions.csv)との全組み合わせ照合
- [x] アウトゲーム: タイトル（準備をする／オプション／クレジット）、Pusher・ステージ・解禁済み難易度の選択、言語設定の保存
- [ ] 属性の特殊効果、物理バランスと実機での性能検証
- [ ] 2.5Dの見た目（現在は仮の図形。アセットは未作成）

操作: A/D・左右キーで照準、Space・クリック・タップの長押しで連射、Fでオート連射切替、Tabまたはメダルボタンで射出メダルを選択します。
進化ゲージが満タンになると時計と物理が止まり、レベルアップ・錬成・New!を選べます。
ステージをクリアすると1つ上の難易度が解禁されます。オプションの言語設定とクリア実績はローカルへ保存されます。

## オフラインファースト

ゲームはネットワーク・ユーザー認証・外部サーバーに依存しません。進行記録はローカルに保存します。

| OS | 既定の保存先 |
|---|---|
| Windows | `%APPDATA%\nudra\progress.sav` |
| macOS | `~/Library/Application Support/nudra/progress.sav` |
| その他 | OSのユーザーデータ領域、無ければ `./save` |

環境変数 `NUDRA_SAVE_DIR` で変更できます。直前の保存は `progress.bak`、読めなかったファイルは `progress.corrupt` として残ります。

## 表示文（i18n）

表示文は `game/locales/<言語>/ui.json` に書きます（コードには書きません）。
文を足すときは `game/src/presentation/keys.rs` の `Ui` にキーを足し、**全言語**のカタログに同じキーを足してください。
整合は `cargo nextest run -p game -E 'binary(presentation_text)'` で確認できます。

## セットアップ

必要なもの:

- Rust stable（1.100以上）
- Rust nightly（`rustfmt.toml` が nightly のオプションを使うため、フォーマットに必要）
- [cargo-nextest](https://nexte.st/)

```sh
rustup toolchain install nightly --component rustfmt
cargo install cargo-nextest --locked
```

## よく使うコマンド

```sh
# テスト（domain・applicationはBevyのビルド不要で速い）
cargo nextest run -p domain -p application

# 全体のテスト（Bevyのビルドを含むため初回は時間がかかる）
cargo nextest run --workspace

# Lint
cargo clippy --workspace --all-targets

# フォーマット（nightly必須）
cargo +nightly fmt --all
```

## コミット前の検証

`.githooks/pre-commit`はCLAUDE.mdの必須手順に従い、nightly rustfmtによる自動整形、警告ゼロのclippy、cargo-nextestを順に実行します。
domain・applicationを常に検証し、game・共通Cargo設定・hook自身にステージ済み変更があればgameも加えます。
未ステージの変更があれば中断し、変更がなければ整形された追跡済みRustファイルをステージしてから検証します。有効化は次のコマンドです。

```sh
git config core.hooksPath .githooks
```

日本語フォントとライセンスは`game/assets/fonts/`、主要ライブラリのライセンスは`game/assets/licenses/`に同梱しています。

## ドキュメント

- [要件定義書](docs/requirements.md)
- [基本設計書](docs/basic-design.md)
