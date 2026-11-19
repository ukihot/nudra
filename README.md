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
  application/   ユースケース（start_game / launch_medal / advance_waves / resolve_physics / level_up）
game/            Bevy + Avian。Infrastructure と Presentation（実行バイナリ nudra）
docs/
  requirements.md   要件定義書
  basic-design.md   基本設計書
```

依存の向きは `game → application → domain`（と `game → domain`）。domainは外側に依存しません。

## 開発状況

- [x] domain: 集約（Stage / GameSession / PusherCharacter）、定義データ、射出・経路・成長ルール
- [x] application: ユースケース一式
- [ ] game: Bevy・Avianでの実装（現在は空の `main` のみ）

## セットアップ

必要なもの:

- Rust stable（1.94以上）
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

## ドキュメント

- [要件定義書](docs/requirements.md)
- [基本設計書](docs/basic-design.md)
