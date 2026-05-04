; Both branches carry distinct volatile stores so O1 cannot fold them into a select.
; entry stores 1 to mem[0x100], loads it back, and compares (val > 0).
; Stored value 1 > 0 is true → the if.then branch is taken → r1 = 42.
define i32 @main() {
entry:
  store volatile i32 1, ptr inttoptr (i64 256 to ptr), align 4
  %val = load volatile i32, ptr inttoptr (i64 256 to ptr), align 4
  %cmp = icmp sgt i32 %val, 0
  br i1 %cmp, label %if.then, label %if.else

if.then:
  store volatile i32 42, ptr inttoptr (i64 512 to ptr), align 4
  ret i32 42

if.else:
  store volatile i32 0, ptr inttoptr (i64 1024 to ptr), align 4
  ret i32 0
}
