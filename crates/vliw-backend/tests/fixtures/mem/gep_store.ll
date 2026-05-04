; Store i64 %val to base[1] where base is a pointer parameter.
; GEP with constant index 1 on i64 → byte offset 8.
define void @gep_store(ptr %base, i64 %val) {
entry:
  %ptr = getelementptr i64, ptr %base, i64 1
  store i64 %val, ptr %ptr
  ret void
}
