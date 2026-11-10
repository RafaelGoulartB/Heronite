//! Settings → Metadata's entry points: the rows of the sources and the
//! changes to them. Every change is one read-modify-write of
//! `local_library/settings.json` (the SteamGridDB key: of the global
//! settings) and returns the settings as saved, like the other setters.

use anyhow::{anyhow, Result};
use heronite_data::models::MetadataSettings;
use heronite_data::settings::{self, EffectiveSettings};
use heronite_data::source_prefs;

use super::MetadataService;
use crate::sources::options::{option_values, set_option_value};
use crate::sources::{rows, OptionValue, SourceContext, SourceDescriptor, SourceRow, SourceStatus, SourceTest};

impl MetadataService {
    fn descriptor(&self, id: &str) -> Result<&SourceDescriptor> {
        self.sources.get(id).ok_or_else(|| anyhow!("unknown metadata source {id}"))
    }

    /// Whether source `id` is on in the saved settings.
    pub fn source_on(&self, id: &str) -> bool {
        source_prefs::is_on(&self.metadata_settings(), id)
    }

    /// The rows of Settings → Metadata (see [`SourceRow`]). Reads the
    /// global settings: call it after a change, not per frame.
    pub fn source_rows(&self, settings: &MetadataSettings) -> Vec<SourceRow> {
        self.sources.rows(&self.paths, settings, &self.tests)
    }

    /// The status of one source (`None`: not registered).
    pub fn source_status(&self, settings: &MetadataSettings, id: &str) -> Option<SourceStatus> {
        let d = self.sources.get(id)?;
        let values = option_values(&self.paths, settings, d);
        Some(rows::status(d, source_prefs::is_on(settings, id), &values, self.tests.get(id, &values)))
    }

    /// Runs the test of `id` with the saved options and remembers the
    /// outcome for its status. `None` when it has no test. Blocking.
    pub fn test_source(&self, id: &str) -> Option<SourceTest> {
        let d = self.sources.get(id)?;
        let ctx = SourceContext::new(&self.paths, &self.http);
        let result = self.sources.test(&ctx, id)?;
        self.tests.record(id, &option_values(&self.paths, ctx.settings(), d), result.ok);
        Some(result)
    }

    /// Turns a source on or off (any id: one this build does not know is
    /// kept for the build that does).
    pub fn set_source_enabled(&self, id: &str, on: bool) -> Result<EffectiveSettings> {
        settings::update_metadata(&self.paths, |m| source_prefs::set_on(m, id, on))
    }

    /// Saves option `key` of source `id` where it is kept.
    pub fn set_source_option(&self, id: &str, key: &str, value: OptionValue) -> Result<EffectiveSettings> {
        set_option_value(&self.paths, self.descriptor(id)?, key, value)
    }

    /// Lets the merge take `field` from source `id`, or not.
    pub fn set_source_field_enabled(&self, id: &str, field: &str, on: bool) -> Result<EffectiveSettings> {
        settings::update_metadata(&self.paths, |m| source_prefs::set_field_on(m, id, field, on))
    }

    /// Moves `id` to place `to` among the rows of its kind (0 = first).
    pub fn move_source(&self, id: &str, to: usize) -> Result<EffectiveSettings> {
        let current = self.metadata_settings();
        let order = self.sources.order_after_move(&current, id, to).ok_or_else(|| anyhow!("{id} cannot be moved"))?;
        settings::update_metadata(&self.paths, |m| m.source_order = order)
    }

    /// The sources `field` is taken from, first to last, as the merge uses them.
    pub fn field_order(&self, settings: &MetadataSettings, field: &str) -> Vec<String> {
        self.sources.field_order(settings, field)
    }

    /// Whether the user set an order for `field`.
    pub fn is_field_customised(&self, settings: &MetadataSettings, field: &str) -> bool {
        source_prefs::is_field_customised(settings, field)
    }

    /// Saves `order` as the user's order for `field`; the sources it leaves
    /// out keep their place after it.
    pub fn set_field_order(&self, field: &str, order: &[String]) -> Result<EffectiveSettings> {
        let list = self.sources.field_list_to_save(&self.metadata_settings(), field, order);
        settings::update_metadata(&self.paths, |m| {
            m.field_priority.insert(field.to_owned(), list);
        })
    }

    /// Back to the order every source move changes.
    pub fn reset_field_order(&self, field: &str) -> Result<EffectiveSettings> {
        settings::update_metadata(&self.paths, |m| source_prefs::reset_field(m, field))
    }
}
