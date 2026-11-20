//! 入力 → 操作の意図（基本設計 6.4, 11章）。キーボード・マウス・タッチを同じ操作に対応付ける。
//!
//! 旋回と発射ボタンは要求ではなく意図（`ControlIntent`）として書く。
//! 押しっぱなしでも要求は増えない。 実際に発射するかは Infrastructure が固定ステップで
//! Domain（クールダウン）に問い合わせて決める。

use bevy::{picking::hover::Hovered, prelude::*};

use crate::{
    port::{AimModel, ControlIntent},
    state::{AppState, GamePhase},
};

/// キー押下中の旋回速度（rad/s）
const AIM_SPEED: f32 = 1.4;
/// タッチのドラッグ量（px）あたりの旋回量（rad）
const TOUCH_AIM_PER_PX: f32 = 0.008;
const AIM_GUIDE_LENGTH: f32 = 3.5;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (write_intent, draw_aim_guide).run_if(in_state(GamePhase::Running)),
    )
    .add_systems(OnEnter(GamePhase::Evolving), clear_manual_intent)
    .add_systems(OnExit(AppState::InGame), release_intent);
}

fn write_intent(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    time: Res<Time>,
    mut intent: ResMut<ControlIntent>,
    buttons: Query<&Hovered>,
) {
    let pressed = |a, b| keys.pressed(a) || keys.pressed(b);
    // +yaw は左回り（画面左）
    let steer = f32::from(u8::from(pressed(KeyCode::ArrowLeft, KeyCode::KeyA)))
        - f32::from(u8::from(pressed(KeyCode::ArrowRight, KeyCode::KeyD)));
    let over_ui = buttons.iter().any(Hovered::get);
    let drag: f32 = if over_ui {
        0.0
    } else {
        touches.iter().map(|t| t.delta().x).sum()
    };
    // 固定ステップが読むまで旋回量を貯める（フレームと固定ステップの回数が一致しなくても失われない）
    intent.aim += steer * AIM_SPEED * time.delta_secs() - drag * TOUCH_AIM_PER_PX;
    if keys.just_pressed(KeyCode::KeyF) {
        intent.auto_fire = !intent.auto_fire;
    }
    intent.fire = keys.pressed(KeyCode::Space)
        || (!over_ui && (mouse.pressed(MouseButton::Left) || touches.iter().next().is_some()));
}

fn clear_manual_intent(mut intent: ResMut<ControlIntent>) {
    intent.aim = 0.0;
    intent.fire = false;
}

fn release_intent(mut intent: ResMut<ControlIntent>) {
    *intent = ControlIntent::default();
}

fn draw_aim_guide(aim: Res<AimModel>, mut gizmos: Gizmos) {
    if aim.direction != Vec3::ZERO {
        let tip = aim.origin + aim.direction * AIM_GUIDE_LENGTH;
        let color = Color::srgb(0.35, 0.95, 0.85);
        // 操作の方向を点列で示す。着地点の物理予測ではなく、床上の照準目印。
        for i in 1..=12 {
            let t = i as f32 / 12.0;
            let p = aim.origin.lerp(tip, t);
            gizmos.sphere(Isometry3d::from_translation(p), 0.025, color);
        }
        let target = Vec3::new(tip.x, 0.03, tip.z);
        let orientation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        for radius in [0.22, 0.29] {
            gizmos.circle(Isometry3d::new(target, orientation), radius, color);
        }
    }
}
