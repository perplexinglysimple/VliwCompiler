define i32 @live_40(i32 %seed) {
entry:
  %v0 = add i32 %seed, 1
  %v1 = add i32 %v0, %seed
  %v2 = add i32 %v1, %seed
  %v3 = add i32 %v2, %seed
  %v4 = add i32 %v3, %seed
  %v5 = add i32 %v4, %seed
  %v6 = add i32 %v5, %seed
  %v7 = add i32 %v6, %seed
  %v8 = add i32 %v7, %seed
  %v9 = add i32 %v8, %seed
  %v10 = add i32 %v9, %seed
  %v11 = add i32 %v10, %seed
  %v12 = add i32 %v11, %seed
  %v13 = add i32 %v12, %seed
  %v14 = add i32 %v13, %seed
  %v15 = add i32 %v14, %seed
  %v16 = add i32 %v15, %seed
  %v17 = add i32 %v16, %seed
  %v18 = add i32 %v17, %seed
  %v19 = add i32 %v18, %seed
  %v20 = add i32 %v19, %seed
  %v21 = add i32 %v20, %seed
  %v22 = add i32 %v21, %seed
  %v23 = add i32 %v22, %seed
  %v24 = add i32 %v23, %seed
  %v25 = add i32 %v24, %seed
  %v26 = add i32 %v25, %seed
  %v27 = add i32 %v26, %seed
  %v28 = add i32 %v27, %seed
  %v29 = add i32 %v28, %seed
  %v30 = add i32 %v29, %seed
  %v31 = add i32 %v30, %seed
  %v32 = add i32 %v31, %seed
  %v33 = add i32 %v32, %seed
  %v34 = add i32 %v33, %seed
  %v35 = add i32 %v34, %seed
  %v36 = add i32 %v35, %seed
  %v37 = add i32 %v36, %seed
  %v38 = add i32 %v37, %seed
  store volatile i32 %v0, ptr inttoptr (i64 256 to ptr), align 4
  store volatile i32 %v1, ptr inttoptr (i64 260 to ptr), align 4
  store volatile i32 %v2, ptr inttoptr (i64 264 to ptr), align 4
  store volatile i32 %v3, ptr inttoptr (i64 268 to ptr), align 4
  store volatile i32 %v4, ptr inttoptr (i64 272 to ptr), align 4
  store volatile i32 %v5, ptr inttoptr (i64 276 to ptr), align 4
  store volatile i32 %v6, ptr inttoptr (i64 280 to ptr), align 4
  store volatile i32 %v7, ptr inttoptr (i64 284 to ptr), align 4
  store volatile i32 %v8, ptr inttoptr (i64 288 to ptr), align 4
  store volatile i32 %v9, ptr inttoptr (i64 292 to ptr), align 4
  store volatile i32 %v10, ptr inttoptr (i64 296 to ptr), align 4
  store volatile i32 %v11, ptr inttoptr (i64 300 to ptr), align 4
  store volatile i32 %v12, ptr inttoptr (i64 304 to ptr), align 4
  store volatile i32 %v13, ptr inttoptr (i64 308 to ptr), align 4
  store volatile i32 %v14, ptr inttoptr (i64 312 to ptr), align 4
  store volatile i32 %v15, ptr inttoptr (i64 316 to ptr), align 4
  store volatile i32 %v16, ptr inttoptr (i64 320 to ptr), align 4
  store volatile i32 %v17, ptr inttoptr (i64 324 to ptr), align 4
  store volatile i32 %v18, ptr inttoptr (i64 328 to ptr), align 4
  store volatile i32 %v19, ptr inttoptr (i64 332 to ptr), align 4
  store volatile i32 %v20, ptr inttoptr (i64 336 to ptr), align 4
  store volatile i32 %v21, ptr inttoptr (i64 340 to ptr), align 4
  store volatile i32 %v22, ptr inttoptr (i64 344 to ptr), align 4
  store volatile i32 %v23, ptr inttoptr (i64 348 to ptr), align 4
  store volatile i32 %v24, ptr inttoptr (i64 352 to ptr), align 4
  store volatile i32 %v25, ptr inttoptr (i64 356 to ptr), align 4
  store volatile i32 %v26, ptr inttoptr (i64 360 to ptr), align 4
  store volatile i32 %v27, ptr inttoptr (i64 364 to ptr), align 4
  store volatile i32 %v28, ptr inttoptr (i64 368 to ptr), align 4
  store volatile i32 %v29, ptr inttoptr (i64 372 to ptr), align 4
  store volatile i32 %v30, ptr inttoptr (i64 376 to ptr), align 4
  store volatile i32 %v31, ptr inttoptr (i64 380 to ptr), align 4
  store volatile i32 %v32, ptr inttoptr (i64 384 to ptr), align 4
  store volatile i32 %v33, ptr inttoptr (i64 388 to ptr), align 4
  store volatile i32 %v34, ptr inttoptr (i64 392 to ptr), align 4
  store volatile i32 %v35, ptr inttoptr (i64 396 to ptr), align 4
  store volatile i32 %v36, ptr inttoptr (i64 400 to ptr), align 4
  store volatile i32 %v37, ptr inttoptr (i64 404 to ptr), align 4
  store volatile i32 %v38, ptr inttoptr (i64 408 to ptr), align 4
  ret i32 %seed
}
