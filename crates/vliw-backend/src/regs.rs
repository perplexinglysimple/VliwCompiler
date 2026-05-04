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

/// Stack address reserved for saving/restoring the link register across calls.
/// Sits just below the spill area (0x10_000) so it does not overlap spill slots.
pub const LINK_REG_SAVE_ADDR: i64 = 0x0_FFF8;

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
