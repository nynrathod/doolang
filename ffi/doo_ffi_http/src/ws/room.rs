//! Room Registry — Maps room names to sets of connection IDs.
//!
//! Uses `DashMap<String, DashSet<String>>` for lock-free concurrent access.
//! Single source of truth for room membership.
//!
//! Auto-cleanup: When a connection is removed from ConnRegistry,
//! `remove_from_all()` is called to purge it from every room.

use dashmap::{DashMap, DashSet};
use doo_ffi_core::ffi_debug;
use std::sync::OnceLock;

/// Room registry — thread-safe, lock-free.
pub struct RoomRegistry {
    /// room_name → set of connection IDs
    rooms: DashMap<String, DashSet<String>>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self { rooms: DashMap::new() }
    }

    /// Add a connection to a room.
    pub fn join(&self, room: &str, conn_id: &str) {
        let members = self.rooms
            .entry(room.to_string())
            .or_insert_with(DashSet::new);
        members.insert(conn_id.to_string());
        ffi_debug!("WS", "Room '{}' now has {} members", room, members.len());
    }

    /// Remove a connection from a room.
    pub fn leave(&self, room: &str, conn_id: &str) {
        if let Some(members) = self.rooms.get(room) {
            members.remove(conn_id);
            // Clean up empty rooms
            if members.is_empty() {
                drop(members);
                self.rooms.remove(room);
                ffi_debug!("WS", "Room '{}' removed (empty)", room);
            } else {
                ffi_debug!("WS", "Room '{}' now has {} members", room, members.len());
            }
        }
    }

    /// Remove a connection from ALL rooms (called on disconnect).
    pub fn remove_from_all(&self, conn_id: &str) {
        // Collect room names to avoid holding references during mutation
        let room_names: Vec<String> = self.rooms.iter()
            .filter(|entry| entry.value().contains(conn_id))
            .map(|entry| entry.key().clone())
            .collect();

        for room in &room_names {
            self.leave(room, conn_id);
        }

        if !room_names.is_empty() {
            ffi_debug!("WS", "Removed {} from {} rooms", conn_id, room_names.len());
        }
    }

    /// Get all connection IDs in a room.
    pub fn get_members(&self, room: &str) -> Vec<String> {
        self.rooms
            .get(room)
            .map(|members| members.iter().map(|id| id.clone()).collect())
            .unwrap_or_default()
    }

    /// Get the number of members in a room.
    pub fn room_size(&self, room: &str) -> usize {
        self.rooms.get(room).map(|m| m.len()).unwrap_or(0)
    }

    /// Get total number of rooms.
    pub fn count(&self) -> usize {
        self.rooms.len()
    }
}

/// Global room registry.
static ROOM_REGISTRY: OnceLock<RoomRegistry> = OnceLock::new();

pub fn get_room_registry() -> &'static RoomRegistry {
    ROOM_REGISTRY.get_or_init(RoomRegistry::new)
}
