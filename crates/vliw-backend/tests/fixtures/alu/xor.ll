define void @test_xor(i32 %a, i32 %b) {
entry:
  %r = xor i32 %a, %b
  ret void
}
