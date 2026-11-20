//! Infrastructure と Presentation の境界（Bevy流の依存性逆転、基本設計 5.6）。
//!
//! - [`request`]: Presentation →
//!   Infrastructure。意図（[`ControlIntent`]）と指図（[`SubmitRequest`]）
//! - [`response`]: Infrastructure → Presentation。指図の終端・ドメインイベント・読み取り専用の
//!   `*Model`
//! - [`appearance`]: Infrastructure が付ける見た目の指定。Presentation がアセットに解決する
//!
//! 利用側は `crate::port::X` で参照する（下位モジュールの構成に依存しない）。

mod appearance;
mod request;
mod response;

use bevy::prelude::*;

pub use self::{appearance::*, request::*, response::*};

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<SubmitRequest>()
        .add_message::<RequestOutcome>()
        .add_message::<DomainEventMsg>()
        .add_message::<AbilityCue>()
        .init_resource::<ControlIntent>()
        .init_resource::<CharacterRoster>()
        .init_resource::<StageRoster>()
        .init_resource::<OptionsModel>()
        .init_resource::<HudModel>()
        .init_resource::<EvolutionModel>()
        .init_resource::<MedalModel>()
        .init_resource::<AimModel>();
}
