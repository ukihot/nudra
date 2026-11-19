//! Infrastructure層: 定義データ、Repository実装、ユースケースを呼ぶSystem、Avianとの接続。
//! Presentationを参照しない。外へは `port` を通じてだけ公開する。

pub mod data;
pub mod physics;
pub mod progress;
pub mod requests;
pub mod save;
pub mod session;
pub mod world;

use bevy::prelude::*;

pub(crate) fn plugin(app: &mut App) {
    app.add_plugins((data::plugin, progress::plugin, session::plugin));
}
