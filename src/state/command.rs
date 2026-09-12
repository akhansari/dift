/// Whether an external editor should receive a position from the displayed diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorPosition {
    VisibleHunk,
    Default,
}

/// A transition requests at most one external operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Collect,
    ToggleStage,
    OpenDiscard,
    ConfirmDiscard,
    OpenCommit,
    SubmitCommit,
    OpenEditor(EditorPosition),
    Quit,
}

/// Display invalidation and an optional command produced by a state transition.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    pub redraw: bool,
    pub render: bool,
    pub command: Option<Command>,
}

impl Effects {
    pub(super) fn draw(render: bool) -> Self {
        Self {
            redraw: true,
            render,
            ..Self::default()
        }
    }
}
