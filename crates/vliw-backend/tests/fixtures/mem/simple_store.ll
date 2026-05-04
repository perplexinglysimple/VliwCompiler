; int main(void) { volatile u64 *p = (void*)0x100; *p = 42; return 0; }
define i32 @main() {
entry:
  store volatile i64 42, ptr inttoptr (i64 256 to ptr), align 8
  ret i32 0
}
