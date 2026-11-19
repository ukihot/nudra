//! ドメインイベントへの反応（演出・音）。アセットが揃うまではログのみ（基本設計 8.6）。

use bevy::prelude::*;
use domain::event::DomainEvent;

use crate::port::DomainEventMsg;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, log_events);
}

fn log_events(mut events: MessageReader<DomainEventMsg>) {
    for DomainEventMsg(event) in events.read() {
        match event {
            DomainEvent::GameStarted { .. } | DomainEvent::GameLost { .. } => info!(?event),
            _ => debug!(?event),
        }
    }
}
