//! Calling-convention register policy.
//!
//! These constants name the physical registers that have fixed roles in the
//! VLIW ABI.  ISel and the allocator use these names rather than raw numbers.

/// r0: always reads as zero; writes are discarded.
pub const ZERO_REG: u8 = 0;

/// r1: return-value register (integer return values land here).
pub const RETVAL_REG: u8 = 1;

/// r31: link register (call saves the return address here).
pub const LINK_REG: u8 = 31;

/// First integer argument register (r2); holds argument 0 at a call site.
pub const ARG_REG_FIRST: u8 = 2;

/// Last integer argument register (r9); supports up to 8 arguments.
pub const ARG_REG_LAST: u8 = 9;

/// Number of integer argument registers.
pub const ARG_REG_COUNT: u8 = ARG_REG_LAST - ARG_REG_FIRST + 1;

/// Base address of the register spill area for the given memory size.
///
/// Uses the top 1/16th of memory: `memory_size * 15 / 16`.
/// For the default 64 KiB memory this is `0xF000`.
pub fn spill_base(memory_size: u64) -> i64 {
    memory_size as i64 * 15 / 16
}

/// Address reserved for saving/restoring the link register across calls.
///
/// Placed at [`spill_base`]`(memory_size) - 8`, just below the spill area.
pub fn link_reg_save_addr(memory_size: u64) -> i64 {
    spill_base(memory_size) - 8
}

/// First general-purpose register available for virtual-register allocation.
pub const FIRST_ALLOCATABLE_GPR: u8 = RETVAL_REG + 1;

/// Last general-purpose register available for virtual-register allocation.
pub const LAST_ALLOCATABLE_GPR: u8 = LINK_REG - 1;

/// Number of general-purpose registers available for allocation.
pub const ALLOCATABLE_GPR_COUNT: u8 = LAST_ALLOCATABLE_GPR - FIRST_ALLOCATABLE_GPR + 1;

/// Map a virtual-register index onto the reserved-register-free GPR range.
pub fn allocatable_gpr(index: u32) -> Option<u8> {
    if index < u32::from(ALLOCATABLE_GPR_COUNT) {
        Some(FIRST_ALLOCATABLE_GPR + u8::try_from(index).ok()?)
    } else {
        None
    }
}
