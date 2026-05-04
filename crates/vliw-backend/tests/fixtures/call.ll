; Contains a call instruction — not supported yet.
declare i32 @external()

define i32 @uses_call() {
entry:
  %r = call i32 @external()
  ret i32 %r
}
