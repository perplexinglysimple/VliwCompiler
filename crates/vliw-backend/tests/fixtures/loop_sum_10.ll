; LLVM-8 loop fixture:
;   int sum = 0;
;   for (int i = 0; i < 10; ++i) sum += i;
;   return sum;
define i64 @sum_to_10() #0 {
entry:
  br label %loop

loop:
  %i = phi i64 [ 0, %entry ], [ %i_next, %body ]
  %sum = phi i64 [ 0, %entry ], [ %sum_next, %body ]
  %keep_going = icmp slt i64 %i, 10
  br i1 %keep_going, label %body, label %exit

body:
  %sum_next = add i64 %sum, %i
  %i_next = add i64 %i, 1
  br label %loop

exit:
  ret i64 %sum
}

attributes #0 = { noinline nounwind optnone }
