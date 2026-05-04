; Uses i8 — only i1, i32, and i64 are supported.
define void @narrow(ptr %p) {
entry:
  %v = load i8, ptr %p, align 1
  store i8 %v, ptr %p, align 1
  ret void
}
