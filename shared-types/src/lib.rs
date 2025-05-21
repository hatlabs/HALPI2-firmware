#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use postcard_bindgen::PostcardBindings;


#[derive(Serialize, Deserialize, PostcardBindings)]
pub enum FlashUpdateCommand {
    // Initialize firmware update process
    StartUpdate {
        num_blocks: u32,
        total_size: u32,
        expected_crc32: u32,
    },

    // Upload a block of firmware data
    UploadBlock {
        block_num: u32,
        data: Vec<u8>,
        block_crc: u32,
    },

    // Query overall update status
    GetStatus,

    // Commit the uploaded firmware
    CommitUpdate,

    // Abort the update process
    AbortUpdate,
}

#[derive(Serialize, Deserialize, PostcardBindings)]
pub enum FlashUpdateResponse {
    // Acknowledgment of received block (CRC verified)
    BlockReceived {
        success: bool,
        error_code: Option<u8>,
    },

    // Status response including write queue state
    Status {
        state: FlashUpdateState,
        blocks_written: u32,
        ready_for_more: bool,
        error_details: Option<String>,
    },

    // Acknowledgment of command
    Ack {
        success: bool,
        error_code: Option<u8>,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, PostcardBindings)]
pub enum FlashUpdateState {
    Idle,
    Updating,
    ReadyToCommit,
    WriteError,
    DataError,
    Complete,
}
