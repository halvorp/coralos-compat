; Control-definition binding (coralos doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING; heirloom-bind `cdef`): DATA, read by the host
; (System/Heirloom/Bindings/mac68k/) and handed to the engine; no code.
;
; MacWrite 4.5 (Mac OS 68k): CDEF 2 (procID 32), its vertical document scrollbar whose thumb shows the current page number while
; using the standard Control Manager scrollbar parts (inUpButton 20, inDownButton 21, inPageUp 22, inPageDown 23, inThumb 129) and
; min/max/value contract. Keyed strictly by CDEF resource ID 2, exact byte length 1824, and FNV-1a 64 digest.
(control-definition
  (id "macwrite-4.5-cdef2")
  (resource (id 2) (length 1824) (fnv1a64 "0x6cc07d31998bde0d"))
  (meaning scrollbar))
