use avian3d::prelude::*;
use bevy::{
    ecs::error::{FallbackErrorHandler, error, panic},
    log::LogPlugin,
    prelude::*,
};

/// `RUST_LOG` で上書きできる（基本設計 8.6）。
const LOG_FILTER: &str = if cfg!(debug_assertions) {
    "info,game=debug,application=debug,wgpu=error,naga=warn"
} else {
    "warn"
};

fn main() -> AppExit {
    App::new()
        // debug: 技術エラーを即座に顕在化 / release: 記録して継続（基本設計 8.5）
        .insert_resource(FallbackErrorHandler(if cfg!(debug_assertions) { panic } else { error }))
        .add_plugins(
            DefaultPlugins
                .set(LogPlugin { filter: LOG_FILTER.into(), ..default() })
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "NUDRA".into(), ..default() }),
                    ..default()
                }),
        )
        .add_plugins(PhysicsPlugins::new(FixedPostUpdate))
        .add_plugins(game::plugin)
        .run()
}
