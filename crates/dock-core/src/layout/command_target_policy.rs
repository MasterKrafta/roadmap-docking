use super::*;
use crate::AllowedArea;

impl Layout {







    pub(super) fn validate_constraints_allowed_for_target(
        &self,
        constraints: crate::PanelConstraints,
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        if constraints.allows_area(area) {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: allowed_area_rejection_reason(area),
            })
        }
    }






}

pub(super) fn allowed_area_rejection_reason(area: AllowedArea) -> &'static str {
    if area == AllowedArea::MAIN_DOCK {
        "panel constraints disallow the main dock area"
    } else if area == AllowedArea::FLOATING {
        "panel constraints disallow floating areas"
    } else if area == AllowedArea::ICON_RAIL {
        "panel constraints disallow icon rails"
    } else {
        "panel constraints disallow the requested dock area"
    }
}

