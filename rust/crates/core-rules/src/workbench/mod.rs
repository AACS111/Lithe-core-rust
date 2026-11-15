//! Workbench chrome rules that a host runs when the user rearranges the window.

mod chrome_layout;
mod notification_read;

pub use chrome_layout::{
    reduce as reduce_chrome_layout, reduce_json as reduce_chrome_layout_json, Action, CatalogItem,
    Placement, Reduction, State, Surface, Zone, REDUCE_CHROME_LAYOUT,
};

pub use notification_read::{
    reduce_read_state, reduce_read_state_json, ReadAction, ReadReduction, ReadState,
    REDUCE_NOTIFICATION_READ_STATE,
};
