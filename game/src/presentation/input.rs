//! 入力 → 操作の意図（基本設計 6.4, 11章）。キーボード・マウス・タッチを同じ操作に対応付ける。
//!
//! 旋回と発射ボタンは要求ではなく意図（`ControlIntent`）として書く。
//! 押しっぱなしでも要求は増えない。 実際に発射するかは Infrastructure が固定ステップで
//! Domain（クールダウン）に問い合わせて決める。

use bevy::prelude::*;

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
    .add_systems(OnExit(AppState::InGame), release_intent);
    #[cfg(debug_assertions)]
    app.add_systems(Update, debug_level_up.run_if(in_state(GamePhase::Running)));
}

fn write_intent(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    time: Res<Time>,
    mut intent: ResMut<ControlIntent>,
) {
    let pressed = |a, b| keys.pressed(a) || keys.pressed(b);
    // +yaw は左回り（画面左）
    let steer = f32::from(u8::from(pressed(KeyCode::ArrowLeft, KeyCode::KeyA)))
        - f32::from(u8::from(pressed(KeyCode::ArrowRight, KeyCode::KeyD)));
    let drag: f32 = touches.iter().map(|t| t.delta().x).sum();
    // 固定ステップが読むまで旋回量を貯める（フレームと固定ステップの回数が一致しなくても失われない）
    intent.aim += steer * AIM_SPEED * time.delta_secs() - drag * TOUCH_AIM_PER_PX;
    intent.fire = keys.pressed(KeyCode::Space)
        || mouse.pressed(MouseButton::Left)
        || touches.iter().next().is_some();
}

fn release_intent(mut intent: ResMut<ControlIntent>) {
    *intent = ControlIntent::default();
}

fn draw_aim_guide(aim: Res<AimModel>, mut gizmos: Gizmos) {
    if aim.direction != Vec3::ZERO {
        let tip = aim.origin + aim.direction * AIM_GUIDE_LENGTH;
        gizmos.arrow(aim.origin, tip, Color::srgb(1.0, 0.85, 0.3));
    }
}

/// 【仮】成長の選択方式は未確定のため、開発ビルドだけ数字キーで強化の指図を出せるようにする。
#[cfg(debug_assertions)]
fn debug_level_up(
    keys: Res<ButtonInput<KeyCode>>,
    mut submit: MessageWriter<crate::port::SubmitRequest>,
) {
    use domain::{positive::Positive, session::Upgrade, spec::MedalType};

    use crate::port::{Request, SubmitRequest};
    let choices: [(KeyCode, f32, fn(Positive) -> Upgrade); 3] = [
        (KeyCode::Digit1, 1.5, |factor| Upgrade::MedalMass {
            medal: MedalType::Normal,
            factor,
        }),
        (KeyCode::Digit2, 1.25, |factor| Upgrade::PusherSpeed { factor }),
        (KeyCode::Digit3, 1.5, |factor| Upgrade::PusherForce { factor }),
    ];
    for (key, factor, upgrade) in choices {
        if keys.just_pressed(key)
            && let Ok(factor) = Positive::new(factor)
        {
            submit.write(SubmitRequest(Request::LevelUp(upgrade(factor))));
        }
    }
}
