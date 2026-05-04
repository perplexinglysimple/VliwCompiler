; Two-function calling convention fixture.
;
; @add_one is a leaf helper: returns %x + 1 via r1 (RETVAL_REG = 1).
; @caller calls @add_one and forwards the result as its own return value.
;
; Done criteria (LATER-2):
;   - result flows through r1 (RETVAL_REG)
;   - r31 is saved before the call and restored in the continuation block

define i64 @add_one(i64 %x) {
entry:
  %result = add i64 %x, 1
  ret i64 %result
}

define i64 @caller(i64 %x) {
entry:
  %y = call i64 @add_one(i64 %x)
  ret i64 %y
}
