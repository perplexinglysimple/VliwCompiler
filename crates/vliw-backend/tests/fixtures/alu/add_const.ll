define void @test_add_const(i32 %a) {
entry:
  %r = add i32 %a, 42
  ret void
}
