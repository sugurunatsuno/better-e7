use std::{collections::BTreeMap, error::Error, fmt};

use crate::{Dispatcher, GameId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDescriptor {
    id: GameId,
    display_name: String,
}

impl GameDescriptor {
    pub fn new(id: GameId, display_name: impl Into<String>) -> Result<Self, DescriptorError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DescriptorError::EmptyDisplayName);
        }
        Ok(Self { id, display_name })
    }

    #[must_use]
    pub const fn id(&self) -> &GameId {
        &self.id
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

pub trait GamePlugin: Send + Sync {
    fn descriptor(&self) -> &GameDescriptor;
    fn create_dispatcher(&self) -> Result<Dispatcher, PluginError>;
}

#[derive(Default)]
pub struct GameRegistry {
    plugins: BTreeMap<GameId, Box<dyn GamePlugin>>,
}

impl GameRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            plugins: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, plugin: impl GamePlugin + 'static) -> Result<(), RegistryError> {
        let id = plugin.descriptor().id().clone();
        if self.plugins.contains_key(&id) {
            return Err(RegistryError::DuplicateGame(id));
        }
        self.plugins.insert(id, Box::new(plugin));
        Ok(())
    }

    #[must_use]
    pub fn get(&self, id: &GameId) -> Option<&dyn GamePlugin> {
        self.plugins.get(id).map(Box::as_ref)
    }

    pub fn descriptors(&self) -> impl ExactSizeIterator<Item = &GameDescriptor> {
        self.plugins.values().map(|plugin| plugin.descriptor())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorError {
    EmptyDisplayName,
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDisplayName => formatter.write_str("game display name must not be empty"),
        }
    }
}

impl Error for DescriptorError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError(pub String);

impl fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PluginError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateGame(GameId),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGame(id) => write!(formatter, "game is already registered: {id}"),
        }
    }
}

impl Error for RegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StateId;

    struct MockPlugin {
        descriptor: GameDescriptor,
    }

    impl MockPlugin {
        fn new(id: &str, name: &str) -> Self {
            Self {
                descriptor: GameDescriptor::new(GameId::new(id).unwrap(), name).unwrap(),
            }
        }
    }

    impl GamePlugin for MockPlugin {
        fn descriptor(&self) -> &GameDescriptor {
            &self.descriptor
        }

        fn create_dispatcher(&self) -> Result<Dispatcher, PluginError> {
            Ok(Dispatcher::new(StateId::new("unknown").unwrap()))
        }
    }

    #[test]
    fn registers_multiple_games_in_stable_id_order() {
        let mut registry = GameRegistry::new();
        registry
            .register(MockPlugin::new("game-b", "Game B"))
            .unwrap();
        registry
            .register(MockPlugin::new("game-a", "Game A"))
            .unwrap();

        let ids = registry
            .descriptors()
            .map(|descriptor| descriptor.id().as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["game-a", "game-b"]);
        assert!(registry.get(&GameId::new("game-a").unwrap()).is_some());
    }

    #[test]
    fn rejects_a_duplicate_game_id() {
        let mut registry = GameRegistry::new();
        registry
            .register(MockPlugin::new("game-a", "Game A"))
            .unwrap();

        assert_eq!(
            registry
                .register(MockPlugin::new("game-a", "Another Game"))
                .unwrap_err(),
            RegistryError::DuplicateGame(GameId::new("game-a").unwrap())
        );
    }
}
