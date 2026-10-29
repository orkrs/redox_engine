;; Guard example for host API v2.
;;
;; Every update the guard looks up the entity named "player", reads its own id from the input
;; "self_id" and both positions. It outputs the distance as "distance" and the number of entities
;; within 10 m as "allies" (an overlap sphere around itself). Closer than 5 m it emits the event
;; "alert" carrying the distance. Without a player (or without a position) it does nothing.
;;
;; Imports used: find_by_name, get_f32, position_of, overlap_sphere (permissions `query`), and
;; the v1 functions set_f32 and emit_event.
(module
  (import "env" "find_by_name" (func $find (param i32 i32) (result i32)))
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "position_of" (func $pos (param i32 i32) (result i32)))
  (import "env" "overlap_sphere" (func $sphere (param f32 f32 f32 f32 i32 i32) (result i32)))
  (import "env" "emit_event" (func $emit (param i32 i32 f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))

  (memory (export "memory") 1)
  ;; names
  (data (i32.const 0) "player")
  (data (i32.const 8) "self_id")
  (data (i32.const 16) "alert")
  (data (i32.const 24) "distance")
  (data (i32.const 32) "allies")
  ;; memory layout: guard position at 200, player position at 212, overlap ids at 300

  (func (export "on_update") (param f32)
    (local $pid i32)
    (local $sid i32)
    (local $d f32)
    (local $dx f32)
    (local $dy f32)
    (local $dz f32)
    (local.set $pid (call $find (i32.const 0) (i32.const 6)))
    (if (i32.lt_s (local.get $pid) (i32.const 0)) (then (return)))
    (local.set $sid (i32.trunc_f32_s (call $get (i32.const 8) (i32.const 7))))
    (if (i32.eqz (call $pos (local.get $sid) (i32.const 200))) (then (return)))
    (if (i32.eqz (call $pos (local.get $pid) (i32.const 212))) (then (return)))
    (local.set $dx (f32.sub (f32.load (i32.const 212)) (f32.load (i32.const 200))))
    (local.set $dy (f32.sub (f32.load (i32.const 216)) (f32.load (i32.const 204))))
    (local.set $dz (f32.sub (f32.load (i32.const 220)) (f32.load (i32.const 208))))
    (local.set $d
      (f32.sqrt
        (f32.add
          (f32.add (f32.mul (local.get $dx) (local.get $dx)) (f32.mul (local.get $dy) (local.get $dy)))
          (f32.mul (local.get $dz) (local.get $dz)))))
    (call $set (i32.const 24) (i32.const 8) (local.get $d))
    (if (f32.lt (local.get $d) (f32.const 5))
      (then (call $emit (i32.const 16) (i32.const 5) (local.get $d))))
    (call $set (i32.const 32) (i32.const 6)
      (f32.convert_i32_s
        (call $sphere
          (f32.load (i32.const 200))
          (f32.load (i32.const 204))
          (f32.load (i32.const 208))
          (f32.const 10)
          (i32.const 300)
          (i32.const 64))))))
