//! 2.5Dの固定カメラと照明。
//! 縦長の盤面を急角度で見下ろし、画面下（+Z）にカタパルト、画面上（-Z）から敵が来る。

use bevy::prelude::*;

/// 盤面中央のやや奥を、手前上空から見下ろす（約60°）。
const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 25.0, 14.0);
const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 0.0, -0.8);
/// 狭い画角で遠近感を抑え、2.5Dらしい見た目にする。
const CAMERA_FOV_DEG: f32 = 32.0;

pub(super) fn plugin(app: &mut App) {
    app.insert_resource(ClearColor(Color::srgb(0.02, 0.03, 0.02)))
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.55, 0.65, 0.5),
            brightness: 250.0,
            ..default()
        })
        .add_systems(Startup, spawn_camera_and_light);
}

fn spawn_camera_and_light(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: CAMERA_FOV_DEG.to_radians(),
            ..default()
        }),
        Transform::from_translation(CAMERA_POSITION).looking_at(CAMERA_TARGET, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 7_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(-6.0, 14.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
