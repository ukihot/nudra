//! 深海の筐体の構図と照明。設定の正本は render.profiles.ron。

use bevy::{
    anti_alias::{smaa::Smaa, taa::TemporalAntiAliasing},
    camera::Hdr,
    core_pipeline::{
        prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass},
        tonemapping::Tonemapping,
    },
    light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap},
    pbr::{ContactShadows, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel},
    post_process::bloom::{Bloom, BloomPrefilter},
    prelude::*,
    render::{camera::TemporalJitter, view::ColorGrading},
};

use super::{
    keys,
    render_profile::{AntiAliasing, RenderProfile, Tone},
};
use crate::{port::OptionsModel, ron_asset};

#[derive(Resource)]
struct ProfileHandle(Handle<RenderProfile>);

#[derive(Component)]
struct ArtCamera;

#[derive(Component)]
struct KeyLight;

struct ProfileWatch {
    timer: Timer,
    modified: Option<std::time::SystemTime>,
}

impl Default for ProfileWatch {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.5, TimerMode::Repeating),
            modified: None,
        }
    }
}

/// 開発ビルド限定の一時比較値。ユーザー設定には保存しない。
#[derive(Resource, Default)]
struct DebugLook {
    fov_offset: f32,
    environment_scale: f32,
    cool_key: bool,
    disable_bloom: bool,
    experimental: bool,
    tone: Option<Tone>,
}

pub(super) fn plugin(app: &mut App) {
    ron_asset::register::<RenderProfile>(app, keys::RENDER_PROFILE_EXTENSIONS);
    app.init_resource::<RenderProfile>()
        .init_resource::<DebugLook>()
        .add_systems(Startup, load_profile)
        .add_systems(
            Update,
            (watch_profile, refresh_profile, debug_look, apply_profile).chain(),
        );
}

/// このファイルだけを開発時に監視し、再読込はBevyの通常のアセット経路へ渡す。
fn watch_profile(time: Res<Time<Real>>, server: Res<AssetServer>, mut watch: Local<ProfileWatch>) {
    if !cfg!(debug_assertions) || !watch.timer.tick(time.delta()).just_finished() {
        return;
    }
    let Ok(modified) = std::fs::metadata(keys::RENDER_PROFILE_SOURCE).and_then(|m| m.modified())
    else {
        return;
    };
    if watch
        .modified
        .is_some_and(|previous| previous != modified)
    {
        server.reload(keys::RENDER_PROFILE);
    }
    watch.modified = Some(modified);
}

fn load_profile(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ProfileHandle(server.load(keys::RENDER_PROFILE)));
    // HUDは後処理の後に描き、品質設定やHDRで文字・アイコンの見え方を変えない。
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        IsDefaultUiCamera,
        Msaa::Off,
    ));
}

fn refresh_profile(
    handle: Res<ProfileHandle>,
    profiles: Res<Assets<RenderProfile>>,
    mut commands: Commands,
) {
    if !profiles.is_changed() {
        return;
    }
    if let Some(p) = profiles.get(&handle.0) {
        if p.is_valid() {
            commands.insert_resource(p.clone());
        } else {
            warn!(message = keys::INVALID_RENDER_PROFILE);
        }
    }
}

fn debug_look(keys: Res<ButtonInput<KeyCode>>, mut look: ResMut<DebugLook>) {
    if !cfg!(debug_assertions) {
        return;
    }
    if keys.just_pressed(KeyCode::F6) {
        look.fov_offset = if look.fov_offset >= 2.0 {
            -2.0
        } else {
            look.fov_offset + 2.0
        };
    }
    if keys.just_pressed(KeyCode::F7) {
        look.environment_scale = if look.environment_scale >= 0.5 {
            -0.25
        } else {
            look.environment_scale + 0.25
        };
    }
    if keys.just_pressed(KeyCode::F8) {
        look.cool_key = !look.cool_key;
    }
    if keys.just_pressed(KeyCode::F9) {
        look.disable_bloom = !look.disable_bloom;
    }
    if keys.just_pressed(KeyCode::F10) {
        look.experimental = !look.experimental;
    }
    if keys.just_pressed(KeyCode::F11) {
        look.tone = match look.tone {
            None => Some(Tone::BlenderFilmic),
            Some(Tone::BlenderFilmic) => Some(Tone::KhronosPbrNeutral),
            _ => None,
        };
    }
    if keys.just_pressed(KeyCode::F12) {
        *look = DebugLook::default();
    }
}

fn color(rgb: [f32; 3]) -> Color {
    Color::srgb(rgb[0], rgb[1], rgb[2])
}

#[allow(clippy::too_many_arguments)]
fn apply_profile(
    p: Res<RenderProfile>,
    options: Res<OptionsModel>,
    debug: Res<DebugLook>,
    cameras: Query<Entity, With<ArtCamera>>,
    lights: Query<Entity, With<KeyLight>>,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    if !p.is_changed() && !options.is_changed() && !debug.is_changed() && !cameras.is_empty() {
        return;
    }
    let q = if debug.experimental {
        &p.experimental
    } else {
        p.quality(options.render_quality)
    };
    commands.insert_resource(ClearColor(color(p.background)));
    commands.insert_resource(GlobalAmbientLight { brightness: 0.0, ..default() });
    commands.insert_resource(DirectionalLightShadowMap { size: q.shadow_map_size });
    let intensity = p.environment_intensity * (1.0 + debug.environment_scale);
    let environment = match (&p.diffuse_map, &p.specular_map) {
        (Some(diffuse), Some(specular)) => EnvironmentMapLight {
            diffuse_map: server.load(diffuse.clone()),
            specular_map: server.load(specular.clone()),
            intensity,
            ..default()
        },
        _ => EnvironmentMapLight::hemispherical_gradient(
            &mut images,
            color(p.environment_top),
            color(p.environment_horizon),
            color(p.environment_bottom),
        )
        .with_intensity(intensity),
    };
    let camera = cameras
        .iter()
        .next()
        .unwrap_or_else(|| commands.spawn(ArtCamera).id());
    let mut entity = commands.entity(camera);
    // 必須コンポーネントは自動では外れないので、軽量側へ戻す際もすべて明示的に除去する。
    entity.remove::<(
        Bloom,
        Smaa,
        TemporalAntiAliasing,
        TemporalJitter,
        ContactShadows,
        ScreenSpaceAmbientOcclusion,
        DepthPrepass,
        NormalPrepass,
        MotionVectorPrepass,
    )>();
    let mut grading = ColorGrading::default();
    grading.global.exposure = p.exposure;
    grading.global.post_saturation = p.saturation;
    entity.insert((
        Camera3d::default(),
        Hdr,
        Projection::Perspective(PerspectiveProjection {
            fov: (p.fov_deg + debug.fov_offset)
                .clamp(10.0, 80.0)
                .to_radians(),
            ..default()
        }),
        Transform::from_translation(p.camera_position.into())
            .looking_at(p.camera_target.into(), Vec3::Y),
        environment,
        Tonemapping::from(debug.tone.unwrap_or(q.tone)),
        grading,
        if q.anti_aliasing == AntiAliasing::Msaa4 {
            Msaa::Sample4
        } else {
            Msaa::Off
        },
        DistanceFog {
            color: color(p.background),
            falloff: FogFalloff::Linear { start: p.fog_start, end: p.fog_end },
            ..default()
        },
    ));
    match q.anti_aliasing {
        AntiAliasing::Msaa4 => {},
        AntiAliasing::Smaa => {
            entity.insert(Smaa::default());
        },
        AntiAliasing::Taa => {
            entity.insert(TemporalAntiAliasing::default());
        },
    }
    if q.bloom && !debug.disable_bloom {
        entity.insert(Bloom {
            intensity: p.bloom_intensity,
            prefilter: BloomPrefilter {
                threshold: p.bloom_threshold,
                threshold_softness: 0.2,
            },
            ..Bloom::NATURAL
        });
    }
    if q.contact_shadows {
        entity.insert(ContactShadows {
            linear_steps: q.contact_steps,
            thickness: q.contact_thickness,
            length: q.contact_length,
        });
    }
    if let Some(radius) = q.ssao_radius {
        entity.insert(ScreenSpaceAmbientOcclusion {
            quality_level: ScreenSpaceAmbientOcclusionQualityLevel::Medium,
            radius,
            constant_object_thickness: q.ssao_thickness,
        });
    }
    let light = lights
        .iter()
        .next()
        .unwrap_or_else(|| commands.spawn(KeyLight).id());
    let mut key_color = p.key_color;
    if debug.cool_key {
        key_color.reverse();
    }
    commands.entity(light).insert((
        DirectionalLight {
            color: color(key_color),
            illuminance: p.key_illuminance,
            shadow_maps_enabled: true,
            shadow_depth_bias: q.shadow_depth_bias,
            shadow_normal_bias: q.shadow_normal_bias,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: q.shadow_cascades,
            maximum_distance: q.shadow_distance,
            first_cascade_far_bound: q.shadow_first_cascade,
            ..default()
        }
        .build(),
        Transform::from_translation(p.key_position.into()).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[cfg(test)]
mod tests {
    use domain::progress::RenderQuality;

    use super::*;

    #[test]
    fn invalid_hot_reload_preserves_the_last_valid_profile() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<RenderProfile>()
            .init_resource::<RenderProfile>()
            .add_systems(Update, refresh_profile);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<RenderProfile>>()
            .add(RenderProfile { fov_deg: 35.0, ..RenderProfile::default() });
        app.insert_resource(ProfileHandle(handle.clone()));
        app.update();
        assert_eq!(app.world().resource::<RenderProfile>().fov_deg, 35.0);
        app.world_mut()
            .resource_mut::<Assets<RenderProfile>>()
            .get_mut(&handle)
            .expect(keys::VALID_RENDER_PROFILE)
            .fov_deg = f32::NAN;
        app.update();
        assert_eq!(app.world().resource::<RenderProfile>().fov_deg, 35.0);
    }

    #[test]
    fn switching_from_high_removes_all_expensive_camera_features() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<RenderProfile>()
            .init_resource::<OptionsModel>()
            .init_resource::<DebugLook>()
            .add_systems(Update, apply_profile);
        {
            let mut profile = app.world_mut().resource_mut::<RenderProfile>();
            profile.diffuse_map = None;
            profile.specular_map = None;
        }
        app.world_mut()
            .resource_mut::<OptionsModel>()
            .render_quality = RenderQuality::High;
        app.update();
        let camera = app
            .world_mut()
            .query_filtered::<Entity, With<ArtCamera>>()
            .single(app.world())
            .expect(keys::VALID_RENDER_PROFILE);
        assert!(app.world().get::<ContactShadows>(camera).is_some());
        assert!(
            app.world()
                .get::<ScreenSpaceAmbientOcclusion>(camera)
                .is_some()
        );
        assert!(app.world().get::<Smaa>(camera).is_some());
        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));
        app.world_mut()
            .resource_mut::<OptionsModel>()
            .render_quality = RenderQuality::Mobile;
        app.update();
        assert!(app.world().get::<ContactShadows>(camera).is_none());
        assert!(
            app.world()
                .get::<ScreenSpaceAmbientOcclusion>(camera)
                .is_none()
        );
        assert!(app.world().get::<Smaa>(camera).is_none());
        assert!(app.world().get::<Bloom>(camera).is_none());
        assert!(app.world().get::<DepthPrepass>(camera).is_none());
        assert!(app.world().get::<NormalPrepass>(camera).is_none());
        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Sample4));
        app.world_mut()
            .resource_mut::<OptionsModel>()
            .render_quality = RenderQuality::Standard;
        app.update();
        assert!(app.world().get::<Bloom>(camera).is_some());
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<ArtCamera>>()
                .iter(app.world())
                .count(),
            1
        );
    }
}
