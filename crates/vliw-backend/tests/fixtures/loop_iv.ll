; LLVM-8 loop fixture: count from 0 to n, return n.
; Induction variable %i advances by 1 each iteration.
; Models: for (i64 i = 0; i != n; i++) {}  return n;
define i64 @count_to_n(i64 %n) {
entry:
  br label %loop

loop:
  %i = phi i64 [ 0, %entry ], [ %i_next, %loop ]
  %i_next = add i64 %i, 1
  %done = icmp eq i64 %i_next, %n
  br i1 %done, label %exit, label %loop

exit:
  ret i64 %i_next
}
