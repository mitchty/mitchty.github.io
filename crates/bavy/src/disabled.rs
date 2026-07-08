use bevy::prelude::*;

// I am not keen on this approach but fixing its a future mitch problem 0.19 is
// out its time to update to new hotness.
#[cfg(dev_build)]
#[derive(Resource, Default)]
pub struct DisabledPlugins(pub std::collections::HashSet<String>);

#[cfg(dev_build)]
impl DisabledPlugins {
    /// Returns `true` if `name` is in the disabled set (case-insensitive).
    pub fn contains(&self, name: &str) -> bool {
        self.0.contains(&name.to_lowercase())
    }
}
/// Return `true` if `name` was disabled at runtime. Dev builds only.
#[allow(unused_variables)]
pub fn is_disabled(world: &World, name: &str) -> bool {
    #[cfg(dev_build)]
    {
        world
            .get_resource::<DisabledPlugins>()
            .is_some_and(|d| d.contains(name))
    }
    #[cfg(not(dev_build))]
    {
        false
    }
}
