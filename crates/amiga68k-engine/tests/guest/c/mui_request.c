/* muimaster.library/MUI_RequestA (written for this repo, MIT). The host answers each request in order (RecordingGuestUi::request_answers);
 * results in RES[]. RES[0..2] = three buttons with an Application (answers 0, 1, 2), RES[3] = a lone button, RES[4] = NULL title (the
 * Application's), RES[5] = the same with a reference window, RES[6..8] = no Application (EasyRequest numbering), RES[9] = the window. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define MUIA_Application_Title 0x804281b8u
#define MUIA_Application_Window 0x8042bfe0u
#define MUIA_Window_Open 0x80428aa0u
#define MUIA_Window_Title 0x8042ad3du
#define MUIA_Window_RootObject 0x8042cba5u
#define MUIA_Text_Contents 0x8042f8dcu
#define LVO_MUI_NewObjectA (-30)
#define LVO_MUI_RequestA (-42)
#define REQ(app, win, title, gad, fmt, args) LVOX(mui, LVO_MUI_RequestA, app, win, 0, 0, title, gad, fmt, args)

void _start(void) {
  u32 mui = OPENLIB("muimaster.library");
  u32 intui = OPENLIB("intuition.library");
  u32 tt[4] = { MUIA_Text_Contents, (u32)"Hello", 0, 0 };
  u32 txt = LVOX(mui, LVO_MUI_NewObjectA, 0, 0, 0, 0, (u32)"Text.mui", (u32)tt, 0, 0);
  u32 wt[6] = { MUIA_Window_Title, (u32)"Main", MUIA_Window_RootObject, txt, 0, 0 };
  u32 win = LVOX(mui, LVO_MUI_NewObjectA, 0, 0, 0, 0, (u32)"Window.mui", (u32)wt, 0, 0);
  u32 at[6] = { MUIA_Application_Title, (u32)"Notes", MUIA_Application_Window, win, 0, 0 };
  u32 app = LVOX(mui, LVO_MUI_NewObjectA, 0, 0, 0, 0, (u32)"Application.mui", (u32)at, 0, 0);
  u32 op[4] = { MUIA_Window_Open, 1, 0, 0 };
  LVOX(intui, LVO_intuition_SetAttrsA, 0, 0, 0, 0, win, (u32)op, 0, 0);
  struct __attribute__((packed)) { u32 s; u32 n; } args = { (u32)"notes", 1234 };
  const char *three = "*_Save|_Discard|\33b_Cancel";
  const char *body = "\33cSave \33b%s\33n (%ld bytes)?";
  V(9) = win;
  V(0) = REQ(app, 0, "Quit", three, body, &args);
  V(1) = REQ(app, 0, "Quit", three, body, &args);
  V(2) = REQ(app, 0, "Quit", three, body, &args);
  V(3) = REQ(app, 0, "Quit", "OK", "Done", 0);
  V(4) = REQ(app, 0, 0, "OK|Cancel", "Untitled", 0);
  V(5) = REQ(app, win, "Quit", "OK|Cancel", "Child", 0);
  V(6) = REQ(0, 0, "Easy", "Yes|No|Maybe", "No app", 0);
  V(7) = REQ(0, 0, "Easy", "Yes|No|Maybe", "No app", 0);
  V(8) = REQ(0, 0, "Easy", "OK", "No app lone", 0);
  V(10) = 1;
}
