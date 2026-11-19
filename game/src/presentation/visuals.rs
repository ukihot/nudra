//! `Appearance` を実際の見た目に解決する。
//!
//! 見た目は `assets/visuals.visuals.ron` で key ごとに選ぶ。アセットが揃ったら
//! このファイルの該当行を書き換えるだけで差し替わる（コード変更不要・ホットリロード対応）:
//! - `Placeholder`: 仮の図形（円柱・箱・カプセル）を単色で表示
//! - `Billboard`: 2.5D用。カメラを向く板ポリに画像を貼る
//! - `Model`: glTFシーン（例: `"models/rabbit.glb#Scene0"`）

use std::collections::HashMap;

use bevy::prelude::*;
use serde::Deserialize;

use crate::{
    port::{Appearance, Silhouette},
    ron_asset,
};

const MANIFEST: &str = "visuals.visuals.ron";

#[derive(Asset, TypePath, Deserialize, Debug)]
pub struct VisualManifest {
    default: Look,
    entries: HashMap<String, Look>,
}

#[derive(Deserialize, Debug, Clone)]
enum Look {
    Placeholder { color: (f32, f32, f32) },
    Billboard { image: String, height: f32 },
    Model { scene: String },
}

/// 定義ファイルが未読込のときに使う見た目。
const FALLBACK: Look = Look::Placeholder { color: (0.6, 0.6, 0.6) };

#[derive(Resource)]
struct Manifest(Handle<VisualManifest>);

/// 同じ形・同じ見た目のアセットは共有する（大量のメダルで描画をまとめるため）。
#[derive(Resource, Default)]
struct Cache {
    meshes: HashMap<[u32; 4], Handle<Mesh>>,
    materials: HashMap<String, Handle<StandardMaterial>>,
}

/// カメラを向き続ける板ポリ。
#[derive(Component)]
struct Billboard {
    /// 親の中心からの高さ
    lift: f32,
}

pub(super) fn plugin(app: &mut App) {
    ron_asset::register::<VisualManifest>(app, &["visuals.ron"]);
    app.init_resource::<Cache>()
        .add_systems(Startup, load_manifest)
        .add_observer(attach_visual)
        .add_systems(PostUpdate, face_camera.before(TransformSystems::Propagate));
}

fn load_manifest(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Manifest(server.load(MANIFEST)));
}

#[allow(clippy::too_many_arguments)]
fn attach_visual(
    add: On<Add<Appearance>>,
    appearances: Query<&Appearance>,
    manifest: Option<Res<Manifest>>,
    manifests: Res<Assets<VisualManifest>>,
    server: Res<AssetServer>,
    mut cache: ResMut<Cache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let entity = add.entity;
    let Ok(appearance) = appearances.get(entity) else { return };
    let look = manifest
        .and_then(|m| manifests.get(&m.0))
        .map(|m| m.entries.get(appearance.key.as_ref()).unwrap_or(&m.default).clone())
        .unwrap_or(FALLBACK);

    match look {
        Look::Placeholder { color: (r, g, b) } => {
            let mesh = cache
                .meshes
                .entry(silhouette_key(appearance.silhouette))
                .or_insert_with(|| meshes.add(placeholder_mesh(appearance.silhouette)))
                .clone();
            let material = cache
                .materials
                .entry(appearance.key.to_string())
                .or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(r, g, b),
                        perceptual_roughness: 0.8,
                        ..default()
                    })
                })
                .clone();
            commands.entity(entity).insert((Mesh3d(mesh), MeshMaterial3d(material)));
        },
        Look::Billboard { image, height } => {
            let mesh = cache
                .meshes
                .entry(silhouette_key(Silhouette::Cuboid { size: Vec3::new(height, height, 0.0) }))
                .or_insert_with(|| meshes.add(Rectangle::new(height, height)))
                .clone();
            let material = cache
                .materials
                .entry(image.clone())
                .or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color_texture: Some(server.load(image)),
                        alpha_mode: AlphaMode::Blend,
                        unlit: true,
                        cull_mode: None,
                        ..default()
                    })
                })
                .clone();
            // 足元を接地させるため、板の中心を外形の底から height/2 上げる
            let lift = height / 2.0 - appearance.silhouette.height() / 2.0;
            commands.entity(entity).with_child((
                Billboard { lift },
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
            ));
        },
        Look::Model { scene } => {
            commands.entity(entity).with_child(WorldAssetRoot(server.load(scene)));
        },
    }
}

fn placeholder_mesh(silhouette: Silhouette) -> Mesh {
    match silhouette {
        Silhouette::Cylinder { radius, height } => Cylinder::new(radius, height).into(),
        Silhouette::Cuboid { size } => Cuboid::from_size(size).into(),
        Silhouette::Capsule { radius, length } => Capsule3d::new(radius, length).into(),
    }
}

/// 形と寸法が同じならメッシュを共有するためのキー。
fn silhouette_key(silhouette: Silhouette) -> [u32; 4] {
    match silhouette {
        Silhouette::Cylinder { radius, height } => [0, radius.to_bits(), height.to_bits(), 0],
        Silhouette::Cuboid { size } => [1, size.x.to_bits(), size.y.to_bits(), size.z.to_bits()],
        Silhouette::Capsule { radius, length } => [2, radius.to_bits(), length.to_bits(), 0],
    }
}

/// 板ポリを、親の回転（転がるメダルなど）を打ち消してカメラへ向ける。
fn face_camera(
    camera: Query<&Transform, (With<Camera3d>, Without<Billboard>)>,
    parents: Query<&Transform, Without<Billboard>>,
    mut billboards: Query<(&Billboard, &ChildOf, &mut Transform)>,
) {
    let Ok(camera) = camera.single() else { return };
    for (billboard, child_of, mut transform) in &mut billboards {
        let Ok(parent) = parents.get(child_of.parent()) else { continue };
        let inverse = parent.rotation.inverse();
        transform.rotation = inverse * camera.rotation;
        transform.translation = inverse * (Vec3::Y * billboard.lift);
    }
}
