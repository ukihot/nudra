//! NUDRA のゲーム本体（Infrastructure + Presentation）。
//!
//! 依存性逆転は Bevy ECS の流儀で行う（基本設計 5.6）:
//! - Infrastructure と Presentation は互いを参照しない。
//! - 両者は [`port`] の Message・読み取り専用 Resource・Component だけを介して連携する。
//! - ユースケース（application）は Infrastructure の System からだけ呼ぶ。

pub mod infrastructure;
pub mod port;
pub mod presentation;
pub mod ron_asset;
pub mod state;

use bevy::prelude::*;

/// 描画を伴わない中核（状態・ポート・Infrastructure）。ヘッドレスのテストでも使う。
/// `PhysicsPlugins` と、`AssetPlugin` などBevyの基盤Pluginは呼び出し側で追加する。
pub fn core_plugin(app: &mut App) {
    app.add_plugins((state::plugin, port::plugin, infrastructure::plugin));
}

/// ゲーム全体（中核 + Presentation）。
pub fn plugin(app: &mut App) {
    app.add_plugins((core_plugin, presentation::plugin));
}
