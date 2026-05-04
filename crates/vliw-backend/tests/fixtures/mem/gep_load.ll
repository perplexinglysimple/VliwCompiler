; Load i64 from base[2] where base is a pointer parameter.
; GEP with constant index 2 on i64 → byte offset 16.
define i64 @gep_load(ptr %base) {
entry:
  %ptr = getelementptr i64, ptr %base, i64 2
  %v = load i64, ptr %ptr
  ret i64 %v
}
