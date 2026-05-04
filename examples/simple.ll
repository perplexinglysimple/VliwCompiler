; ModuleID = 'simple.c'
source_filename = "simple.c"

define i32 @main() {
entry:
  store volatile i64 42, ptr inttoptr (i64 256 to ptr), align 8
  ret i32 0
}
