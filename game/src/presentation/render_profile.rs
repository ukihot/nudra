//! RON の描画設定と検証。保存する選択と、実機用の描画値を分離する。

use bevy::{core_pipeline::tonemapping::Tonemapping, prelude::*};
use domain::progress::RenderQuality;
use serde::Deserialize;

use super::keys;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
pub(super) enum AntiAliasing {
    Msaa4,
    Smaa,
    Taa,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub(super) enum Tone {
    AgX,
    BlenderFilmic,
    KhronosPbrNeutral,
}

impl From<Tone> for Tonemapping {
    fn from(tone: Tone) -> Self {
        match tone {
            Tone::AgX => Self::AgX,
            Tone::BlenderFilmic => Self::BlenderFilmic,
            Tone::KhronosPbrNeutral => Self::KhronosPbrNeutral,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct QualityProfile {
    pub shadow_map_size: usize,
    pub shadow_cascades: usize,
    pub shadow_distance: f32,
    pub shadow_first_cascade: f32,
    pub shadow_depth_bias: f32,
    pub shadow_normal_bias: f32,
    pub anti_aliasing: AntiAliasing,
    pub bloom: bool,
    pub contact_shadows: bool,
    pub contact_length: f32,
    pub contact_thickness: f32,
    pub contact_steps: u32,
    pub ssao_radius: Option<f32>,
    pub ssao_thickness: f32,
    pub tone: Tone,
}

#[derive(Asset, TypePath, Deserialize, Debug, Resource, Clone)]
pub(super) struct RenderProfile {
    pub camera_position: [f32; 3],
    pub camera_target: [f32; 3],
    pub fov_deg: f32,
    pub background: [f32; 3],
    pub environment_top: [f32; 3],
    pub environment_horizon: [f32; 3],
    pub environment_bottom: [f32; 3],
    pub environment_intensity: f32,
    pub diffuse_map: Option<String>,
    pub specular_map: Option<String>,
    pub key_color: [f32; 3],
    pub key_illuminance: f32,
    pub key_position: [f32; 3],
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub saturation: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub mobile: QualityProfile,
    pub standard: QualityProfile,
    pub high: QualityProfile,
    pub experimental: QualityProfile,
}

impl Default for RenderProfile {
    fn default() -> Self {
        ron::from_str(keys::BUNDLED_RENDER_PROFILE).expect(keys::VALID_RENDER_PROFILE)
    }
}

impl RenderProfile {
    pub fn quality(&self, quality: RenderQuality) -> &QualityProfile {
        match quality {
            RenderQuality::Mobile => &self.mobile,
            RenderQuality::Standard => &self.standard,
            RenderQuality::High => &self.high,
        }
    }

    pub fn is_valid(&self) -> bool {
        let forward = Vec3::from(self.camera_target) - Vec3::from(self.camera_position);
        let key = Vec3::from(self.key_position);
        let colors = [
            self.background,
            self.environment_top,
            self.environment_horizon,
            self.environment_bottom,
            self.key_color,
        ];
        let map_pair = match (&self.diffuse_map, &self.specular_map) {
            (None, None) => true,
            (Some(a), Some(b)) => !a.trim().is_empty() && !b.trim().is_empty(),
            _ => false,
        };
        self.camera_position
            .into_iter()
            .chain(self.camera_target)
            .all(f32::is_finite)
            && forward.cross(Vec3::Y).length_squared() > 0.001
            && key.is_finite()
            && key.cross(Vec3::Y).length_squared() > 0.001
            && (10.0..=80.0).contains(&self.fov_deg)
            && colors
                .into_iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
            && positive(self.environment_intensity)
            && positive(self.key_illuminance)
            && (0.0..=1.0).contains(&self.bloom_intensity)
            && self.bloom_threshold.is_finite()
            && self.bloom_threshold >= 0.0
            && (-10.0..=10.0).contains(&self.exposure)
            && (0.0..=2.0).contains(&self.saturation)
            && self.fog_start.is_finite()
            && self.fog_start >= 0.0
            && self.fog_end.is_finite()
            && self.fog_end > self.fog_start
            && map_pair
            && [&self.mobile, &self.standard, &self.high, &self.experimental]
                .into_iter()
                .all(QualityProfile::is_valid)
    }
}

fn positive(v: f32) -> bool {
    v.is_finite() && v > 0.0
}

impl QualityProfile {
    fn is_valid(&self) -> bool {
        (256..=8192).contains(&self.shadow_map_size)
            && self.shadow_map_size.is_power_of_two()
            && (1..=4).contains(&self.shadow_cascades)
            && self.shadow_distance.is_finite()
            && self.shadow_distance > 0.1
            && self.shadow_first_cascade.is_finite()
            && self.shadow_first_cascade > 0.1
            && self.shadow_first_cascade < self.shadow_distance
            && self.shadow_depth_bias.is_finite()
            && self.shadow_depth_bias >= 0.0
            && self.shadow_normal_bias.is_finite()
            && self.shadow_normal_bias >= 0.0
            && positive(self.contact_length)
            && positive(self.contact_thickness)
            && (1..=64).contains(&self.contact_steps)
            && positive(self.ssao_thickness)
            && self.ssao_radius.is_none_or(positive)
            && (self.ssao_radius.is_none() || self.anti_aliasing != AntiAliasing::Msaa4)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn bundled_profiles_satisfy_gpu_constraints() {
        let p = RenderProfile::default();
        assert!(p.is_valid());
        assert_eq!(p.mobile.anti_aliasing, AntiAliasing::Msaa4);
        assert!(!p.mobile.bloom && p.mobile.ssao_radius.is_none());
        assert_eq!(p.high.anti_aliasing, AntiAliasing::Smaa);
        assert!(p.high.contact_shadows && p.high.ssao_radius.is_some());
    }

    #[rstest]
    #[case(0.0)]
    #[case(-1.0)]
    #[case(f32::NAN)]
    #[case(f32::INFINITY)]
    #[case(f32::NEG_INFINITY)]
    fn invalid_projection_and_ssao_are_rejected(#[case] value: f32) {
        let p = RenderProfile { fov_deg: value, ..RenderProfile::default() };
        assert!(!p.is_valid());
        let mut p = RenderProfile::default();
        p.high.ssao_radius = Some(value);
        assert!(!p.is_valid());
    }

    #[test]
    fn incompatible_aa_and_degenerate_views_are_rejected() {
        let mut p = RenderProfile::default();
        p.high.anti_aliasing = AntiAliasing::Msaa4;
        assert!(!p.is_valid());
        let mut p = RenderProfile::default();
        p.camera_target = p.camera_position;
        assert!(!p.is_valid());
        let mut p = RenderProfile::default();
        p.standard.shadow_map_size = 0;
        assert!(!p.is_valid());
        let mut p = RenderProfile::default();
        p.standard.shadow_cascades = 0;
        assert!(!p.is_valid());
    }
}
