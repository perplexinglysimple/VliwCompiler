define void @test_ashr(i32 %a, i32 %b) {
entry:
  %r = ashr i32 %a, %b
  ret void
}
