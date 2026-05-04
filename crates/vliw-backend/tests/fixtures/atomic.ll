; Uses atomicrmw — not supported.
define i64 @do_atomic(ptr %p) {
entry:
  %old = atomicrmw add ptr %p, i64 1 seq_cst
  ret i64 %old
}
