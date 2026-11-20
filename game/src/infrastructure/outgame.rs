//! タイトルからの画面遷移、言語の保存、準備画面の読み取りモデル。

use bevy::prelude::*;

use super::{
    data::StageCatalog,
    progress::{Progress, SaveJob},
    requests::{RequestKind, RequestLedger, outcome},
    session::{register_requests, report},
};
use crate::{
    port::{
        OptionsModel, OutgameScreen, RejectReason, Request, RequestOutcome, RequestStatus,
        StageEntry, StageRoster,
    },
    state::AppState,
};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (navigate, set_language, set_render_quality).after(register_requests),
    )
    .add_systems(
        Update,
        publish_models
            .after(set_language)
            .after(set_render_quality),
    );
}

fn navigate(
    mut ledger: ResMut<RequestLedger>,
    state: Res<State<AppState>>,
    mut next: ResMut<NextState<AppState>>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    let (accepted, duplicates) = ledger.take_first(RequestKind::Navigate);
    report(duplicates, &mut outcomes);
    if let Some((id, request)) = accepted {
        let Request::Navigate(screen) = &request else {
            return;
        };
        let target = match (*state.get(), screen) {
            (AppState::Title, OutgameScreen::Preparation) => Some(AppState::CharacterSelect),
            (AppState::Title, OutgameScreen::Options) => Some(AppState::Options),
            (AppState::Title, OutgameScreen::Credits) => Some(AppState::Credits),
            (
                AppState::CharacterSelect | AppState::Options | AppState::Credits,
                OutgameScreen::Title,
            ) => Some(AppState::Title),
            _ => None,
        };
        let status = if let Some(target) = target {
            next.set(target);
            RequestStatus::Completed
        } else {
            RequestStatus::Rejected(RejectReason::WrongState)
        };
        report([outcome(id, request, status)], &mut outcomes);
    }
}

fn set_language(
    mut ledger: ResMut<RequestLedger>,
    state: Res<State<AppState>>,
    mut progress: ResMut<Progress>,
    mut save: ResMut<SaveJob>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    for (id, request) in ledger.take_all(RequestKind::SetLanguage) {
        let Request::SetLanguage(language) = &request else {
            continue;
        };
        let status = if *state.get() == AppState::Options {
            progress.set_language(*language);
            save.request(&progress);
            RequestStatus::Completed
        } else {
            RequestStatus::Rejected(RejectReason::WrongState)
        };
        report([outcome(id, request, status)], &mut outcomes);
    }
}

fn publish_models(
    stages: Option<Res<StageCatalog>>,
    progress: Res<Progress>,
    mut roster: ResMut<StageRoster>,
    mut options: ResMut<OptionsModel>,
) {
    options.set_if_neq(OptionsModel {
        language: progress.language(),
        render_quality: progress.render_quality(),
    });
    if let Some(stages) = stages
        && (stages.is_changed() || progress.is_changed())
    {
        let mut entries: Vec<_> = stages
            .0
            .keys()
            .map(|id| StageEntry {
                id: id.clone(),
                unlocked: progress.unlocked_difficulty(id),
            })
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        roster.set_if_neq(StageRoster(entries));
    }
}

fn set_render_quality(
    mut ledger: ResMut<RequestLedger>,
    state: Res<State<AppState>>,
    mut progress: ResMut<Progress>,
    mut save: ResMut<SaveJob>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    for (id, request) in ledger.take_all(RequestKind::SetRenderQuality) {
        let Request::SetRenderQuality(quality) = &request else {
            continue;
        };
        let status = if *state.get() == AppState::Options {
            progress.set_render_quality(*quality);
            save.request(&progress);
            RequestStatus::Completed
        } else {
            RequestStatus::Rejected(RejectReason::WrongState)
        };
        report([outcome(id, request, status)], &mut outcomes);
    }
}
