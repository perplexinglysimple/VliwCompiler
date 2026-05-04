define void @test_shl(i32 %a, i32 %b) {
entry:
  %r = shl i32 %a, %b
  ret void
}
