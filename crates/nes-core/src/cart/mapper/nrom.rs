//! Mapper 0: NROM. No bank switching.

use super::{Mapper, MapperInit};
use crate::cart::board::Board;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Nrom {
    board: Board,
}

impl Nrom {
    pub fn new(init: MapperInit) -> Self {
        Self { board: init.board }
    }
}

impl Mapper for Nrom {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "NROM"
    }
}
