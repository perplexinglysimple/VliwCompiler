define i32 @if_lt(i32 %x, i32 %y) {
entry:
  %cmp = icmp slt i32 %x, %y
  br i1 %cmp, label %if.then, label %if.else

if.then:
  ret i32 1

if.else:
  ret i32 0
}
