; Uses phi — not supported until LLVM-7.
define i64 @with_phi(i64 %n) {
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
