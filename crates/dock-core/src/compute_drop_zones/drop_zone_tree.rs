use super::*;

#[path = "drop_zone_tree/tree_index.rs"]
mod tree_index;
#[path = "drop_zone_tree/area_queries.rs"]
mod area_queries;
#[path = "drop_zone_tree/main_column_targets.rs"]
mod main_column_targets;
#[path = "drop_zone_tree/group_edge_noop.rs"]
mod group_edge_noop;

use area_queries::main_side_for_node;
pub(crate) use area_queries::{
    is_rail_group, owner_column, source_is_floating_root, target_area,
};
pub(crate) use group_edge_noop::group_edge_is_noop;
pub(crate) use main_column_targets::{
    can_add_column, edge_group_for_column, edge_group_for_dock_edge, group_main_column,
    main_columns,
};
pub(crate) use tree_index::{
    collect_panels, contains_node, direct_parent, panel_node, source_allows, source_root,
    DropZoneTreeIndex,
};
