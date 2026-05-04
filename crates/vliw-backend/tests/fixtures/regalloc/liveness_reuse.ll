; Demonstrates liveness-aware register reuse across 30 definitions.
;
; Each value is used exactly once (in the next definition) and then dead.
; Liveness analysis recognises that only one value is live-out of `entry`
; (%v29), so every other definition's physical register is recycled
; immediately after its single use.  The entire chain shares a single
; computation register (r2); r3 is never needed.
;
; Without liveness-aware freeing all 31 virtual registers would appear
; simultaneously live, requiring more physical registers than the 29
; available and triggering OutOfRegisters.

define i32 @liveness_reuse(i32 %seed) {
entry:
  %v0  = add i32 %seed, %seed
  %v1  = add i32 %v0,  %v0
  %v2  = add i32 %v1,  %v1
  %v3  = add i32 %v2,  %v2
  %v4  = add i32 %v3,  %v3
  %v5  = add i32 %v4,  %v4
  %v6  = add i32 %v5,  %v5
  %v7  = add i32 %v6,  %v6
  %v8  = add i32 %v7,  %v7
  %v9  = add i32 %v8,  %v8
  %v10 = add i32 %v9,  %v9
  %v11 = add i32 %v10, %v10
  %v12 = add i32 %v11, %v11
  %v13 = add i32 %v12, %v12
  %v14 = add i32 %v13, %v13
  %v15 = add i32 %v14, %v14
  %v16 = add i32 %v15, %v15
  %v17 = add i32 %v16, %v16
  %v18 = add i32 %v17, %v17
  %v19 = add i32 %v18, %v18
  %v20 = add i32 %v19, %v19
  %v21 = add i32 %v20, %v20
  %v22 = add i32 %v21, %v21
  %v23 = add i32 %v22, %v22
  %v24 = add i32 %v23, %v23
  %v25 = add i32 %v24, %v24
  %v26 = add i32 %v25, %v25
  %v27 = add i32 %v26, %v26
  %v28 = add i32 %v27, %v27
  %v29 = add i32 %v28, %v28
  br label %exit

exit:
  ret i32 %v29
}
