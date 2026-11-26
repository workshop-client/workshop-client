(module
  (import "workshop" "log" (func $log (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "Hello from Workshop WASM!")
  (func (export "workshop_abi_version") (result i32) i32.const 1)
  (func (export "workshop_start")
    i32.const 0
    i32.const 25
    call $log
    drop))
