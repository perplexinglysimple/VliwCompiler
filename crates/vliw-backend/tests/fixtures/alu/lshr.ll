define void @test_lshr(i32 %a, i32 %b) {
entry:
  %r = lshr i32 %a, %b
  ret void
}
