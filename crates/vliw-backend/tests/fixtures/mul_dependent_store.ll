; Multiplies 3 * 5 = 15 and stores the product to mem[0x100].
; Volatile store/load prevents O1 from constant-folding the multiply,
; so the emitted scalar bundle stream must contain a real mul instruction
; followed by latency-padding nop bundles before the dependent stw.
define void @mul_dependent_store() {
entry:
  store volatile i32 3, ptr inttoptr (i64 512 to ptr), align 4
  %a = load volatile i32, ptr inttoptr (i64 512 to ptr), align 4
  store volatile i32 5, ptr inttoptr (i64 516 to ptr), align 4
  %b = load volatile i32, ptr inttoptr (i64 516 to ptr), align 4
  %product = mul i32 %a, %b
  store volatile i32 %product, ptr inttoptr (i64 256 to ptr), align 4
  ret void
}
