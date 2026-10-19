/* keymap.library through the library ABI, exec RawPutChar, aros.library ArosInquireA, commodities BrokerCommand + the CxMsg readers, and
 * DupLockFromFH on the console handle (written for this repo, MIT). Results in RES[]. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
struct IEvent { u32 next; u8 cls, sub; u16 code, qual; u32 pos; u32 tv[2]; };
struct NewBroker { u8 ver, pad; const char *name, *title, *descr; i16 unique, flags; u8 pri, pad2; u32 port; i16 chan; } __attribute__((packed));
struct KM { u32 p[8]; };
#define AI(n) (0x80000000u + (n))
void _start(void) {
  u32 km = OPENLIB("keymap.library"), dos = OPENLIB("dos.library"), aros = OPENLIB("aros.library"), cx = OPENLIB("commodities.library");
  u8 buf[16]; struct IEvent ev; memset(&ev, 0, sizeof ev); ev.cls = 1; ev.code = 0x20;
  u32 def = LVO(km, LVO_keymap_AskKeyMapDefault, 0, 0, 0, 0, 0, 0);
  V(0) = def != 0; V(1) = def == LVO(km, LVO_keymap_AskKeyMapDefault, 0, 0, 0, 0, 0, 0);
  V(2) = LVOX(km, LVO_keymap_MapRawKey, 0, 8, 0, 0, &ev, buf, 0, 0) & 0xFFFF; V(3) = buf[0];                     /* 'a' */
  ev.qual = 1; LVOX(km, LVO_keymap_MapRawKey, 0, 8, 0, 0, &ev, buf, 0, 0); V(4) = buf[0];                         /* shift: 'A' */
  ev.code = 0x50; ev.qual = 0;
  V(5) = LVOX(km, LVO_keymap_MapRawKey, 0, 8, 0, 0, &ev, buf, 0, 0) & 0xFFFF; V(6) = buf[1];                      /* F1: ESC [ 0 ~ -> 3 bytes, CSI first */
  V(7) = (u32)(i32)(i16)LVOX(km, LVO_keymap_MapRawKey, 0, 1, 0, 0, &ev, buf, 0, 0);                               /* buffer too small: -1 */
  u8 pairs[16];
  V(8) = LVOX(km, LVO_keymap_MapANSI, 3, 8, 0, 0, "Hi!", pairs, 0, 0); V(9) = pairs[0] | (pairs[1] << 8) | (pairs[2] << 16) | (pairs[3] << 24);
  V(20) = LVOX(km, LVO_keymap_MapANSI, 1, 8, 0, 0, "|", pairs, 0, 0);                                              /* no key types '|' in this layout: 0 */
  /* A keymap of the program's own: a copy of the default with the 'a' key typing 'z'. */
  struct KM mine = *(struct KM *)def; u32 lo[64]; memcpy(lo, (void *)mine.p[1], sizeof lo); lo[0x20] = 0x5A7A5A7A; mine.p[1] = (u32)lo;
  ev.code = 0x20;
  LVO(km, LVO_keymap_SetKeyMapDefault, 0, 0, 0, 0, (u32)&mine, 0);
  V(21) = LVO(km, LVO_keymap_AskKeyMapDefault, 0, 0, 0, 0, 0, 0) == (u32)&mine;
  LVOX(km, LVO_keymap_MapRawKey, 0, 8, 0, 0, &ev, buf, 0, 0); V(10) = buf[0];                                      /* 'z' through the new default */
  V(12) = (LVOX(km, LVO_keymap_MapRawKey, 0, 8, 0, 0, &ev, buf, (u32)def, 0), buf[0]);                              /* explicit keymap: 'a' */
  LVO(km, LVO_keymap_SetKeyMapDefault, 0, 0, 0, 0, def, 0);
  LVOX(km, LVO_keymap_MapRawKey, 0, 8, 0, 0, &ev, buf, 0, 0); V(11) = buf[0];                                      /* 'a' again */
  /* exec RawPutChar */
  const char *s = "ser\n"; for (int i = 0; s[i]; i++) LVO(SYSBASE, LVO_exec_RawPutChar, (u32)s[i], 0, 0, 0, 0, 0);
  LVO(SYSBASE, LVO_exec_RawPutChar, 0, 0, 0, 0, 0, 0);                                                              /* NUL: ignored */
  /* aros.library */
  u32 ver = 0, major = 0, kbase = 0xDEAD, abi = 0;
  u32 tl[10] = { AI(1), (u32)&ver, AI(2), (u32)&major, AI(0x10001), (u32)&kbase, AI(8), (u32)&abi, 0, 0 };
  V(13) = LVO(aros, LVO_aros_ArosInquireA, 0, 0, 0, 0, (u32)tl, 0);
  V(14) = ver; V(15) = major; V(16) = kbase; V(17) = abi;
  /* DupLockFromFH(Output()) */
  u32 out = LVO(dos, LVO_dos_Output, 0, 0, 0, 0, 0, 0);
  V(18) = LVO5(dos, LVO_dos_DupLockFromFH, 0, out, 0, 0, 0, 0, 0);
  V(19) = LVO5(dos, LVO_dos_IoErr, 0, 0, 0, 0, 0, 0, 0);
  /* commodities */
  V(22) = LVO(cx, LVO_commodities_BrokerCommand, 15, 0, 0, 0, 0, 0);                                               /* NULL = "Exchange": none -> -1 */
  u32 port = LVO(SYSBASE, LVO_exec_CreateMsgPort, 0, 0, 0, 0, 0, 0);
  struct NewBroker nb = { 5, 0, "TestBroker", "t", "d", 0, 0, 0, 0, port, 0 };
  u32 err = 99; u32 b1 = LVOX(cx, LVO_commodities_CxBroker, (u32)&err, 0, 0, 0, &nb, 0, 0, 0);
  V(23) = b1 != 0 && err == 0;
  V(24) = LVO(cx, LVO_commodities_BrokerCommand, 15, 0, 0, 0, (u32)"TestBroker", 0);                              /* sent */
  u32 msg = LVO(SYSBASE, LVO_exec_GetMsg, 0, 0, 0, 0, port, 0);
  V(25) = msg != 0;
  V(26) = LVO(cx, LVO_commodities_CxMsgType, 0, 0, 0, 0, msg, 0); V(27) = LVO(cx, LVO_commodities_CxMsgID, 0, 0, 0, 0, msg, 0);
  V(28) = LVO(cx, LVO_commodities_CxMsgData, 0, 0, 0, 0, msg, 0);
  LVO(SYSBASE, LVO_exec_ReplyMsg, 0, 0, 0, 0, 0, msg);
  V(29) = LVO(cx, LVO_commodities_BrokerCommand, 15, 0, 0, 0, (u32)"Nope", 0);                                    /* -1 */
  struct NewBroker nb2 = { 5, 0, "NoPort", "t", "d", 0, 0, 0, 0, 0, 0 };
  LVOX(cx, LVO_commodities_CxBroker, (u32)&err, 0, 0, 0, &nb2, 0, 0, 0);
  V(30) = LVO(cx, LVO_commodities_BrokerCommand, 15, 0, 0, 0, (u32)"NoPort", 0);                                  /* no port, not KILL: -2 */
  V(31) = LVO(cx, LVO_commodities_BrokerCommand, 23, 0, 0, 0, (u32)"NoPort", 0);                                  /* KILL: signals this task (CTRL-E), 0 */
  V(32) = (LVO(SYSBASE, LVO_exec_SetSignal, 0, 0, 0, 0, 0, 0) >> 12) & 1;
  CLOSELIB(km); CLOSELIB(dos); CLOSELIB(aros); CLOSELIB(cx);
}
