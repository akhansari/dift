use super::{Command, Effects, State};
use crate::{
    git::{ChangeTarget, DiscardOption, DiscardPreview},
    input::Action,
};
use std::sync::Arc;

pub(crate) struct DiscardMenu {
    pub target: ChangeTarget,
    pub options: Option<[DiscardOption; 2]>,
    pub selected: usize,
    pub executing: bool,
    pub notice: Option<String>,
    pub scroll: usize,
}

impl DiscardMenu {
    pub fn preview(&self) -> Option<&Arc<DiscardPreview>> {
        self.options
            .as_ref()?
            .get(self.selected)?
            .as_ref()
            .ok()
            .filter(|preview| preview.files() > 0)
    }
}

impl State {
    pub(crate) fn discard_menu(&self) -> Option<&DiscardMenu> {
        self.discard.as_ref()
    }

    pub(crate) fn begin_discard(&mut self, target: ChangeTarget) {
        self.discard = Some(DiscardMenu {
            target,
            options: None,
            selected: 2,
            executing: false,
            notice: None,
            scroll: 0,
        });
    }

    pub(crate) fn discard_prepared(&mut self, options: [DiscardOption; 2]) {
        if let Some(menu) = &mut self.discard {
            menu.selected = if menu.notice.is_some() {
                2
            } else {
                options
                    .iter()
                    .position(|option| option.as_ref().is_ok_and(|preview| preview.files() > 0))
                    .unwrap_or(2)
            };
            menu.options = Some(options);
            menu.executing = false;
            menu.scroll = 0;
        }
    }

    pub(crate) fn discard_stale(&mut self) -> Option<ChangeTarget> {
        let menu = self.discard.as_mut()?;
        menu.options = None;
        menu.selected = 2;
        menu.executing = false;
        menu.notice = Some("Changes changed; review again".into());
        Some(menu.target.clone())
    }

    pub(crate) fn take_discard_preview(&mut self) -> Option<Arc<DiscardPreview>> {
        let menu = self.discard.as_mut()?;
        let preview = Arc::clone(menu.preview()?);
        menu.executing = true;
        Some(preview)
    }

    pub(crate) fn close_discard(&mut self) {
        self.discard = None;
    }

    pub(super) fn discard_action(&mut self, action: Action) -> Effects {
        let Some(menu) = &mut self.discard else {
            return Effects::default();
        };
        if menu.executing {
            return Effects::default();
        }
        let mut effects = Effects {
            redraw: true,
            ..Effects::default()
        };
        match action {
            Action::Cancel => self.discard = None,
            Action::Accept if menu.selected == 2 => self.discard = None,
            Action::Accept if menu.preview().is_some() => {
                effects.command = Some(Command::ConfirmDiscard)
            }
            Action::Up if menu.options.is_some() => menu.selected = (menu.selected + 2) % 3,
            Action::Down if menu.options.is_some() => menu.selected = (menu.selected + 1) % 3,
            Action::PageDown => menu.scroll = menu.scroll.saturating_add(4),
            Action::PageUp => menu.scroll = menu.scroll.saturating_sub(4),
            _ => {}
        }
        effects
    }
}
