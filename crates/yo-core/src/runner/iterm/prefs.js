// iTerm2's preferences, for yo: the Default profile (only what JSON can
// hold), whether Claude Code's workgroup has a terminal peer, the
// workgroups themselves (for `yo setup` to add one), and the primary
// display's size. Prints one JSON object. Run with osascript -l JavaScript.
ObjC.import("AppKit");
function plain(v) {
  // What JSON can hold; anything else (NSData, dates) is dropped.
  if (v === null || v === undefined) return undefined;
  var t = typeof v;
  if (t === "string" || t === "number" || t === "boolean") return v;
  if (Array.isArray(v)) { var a = []; v.forEach(function (x) { var p = plain(x); if (p !== undefined) a.push(p); }); return a; }
  if (t === "object" && Object.getPrototypeOf(v) === Object.prototype) {
    var o = {}; Object.keys(v).forEach(function (k) { var p = plain(v[k]); if (p !== undefined) o[k] = p; }); return o;
  }
  return undefined;
}
function run() {
  var d = $.NSUserDefaults.alloc.initWithSuiteName("com.googlecode.iterm2");
  var out = {};
  var books = ObjC.deepUnwrap(d.objectForKey("New Bookmarks")) || [];
  books.forEach(function (b) { if (b && b.Name === "Default") out.default = plain(b); });
  // The workgroups are JSON, kept as bytes. Does Claude Code's have a
  // terminal peer (a shell beside the chat)?
  out.peer = false;
  try {
    var raw = d.objectForKey("Workgroups");
    var text = raw.isKindOfClass($.NSData)
      ? $.NSString.alloc.initWithDataEncoding(raw, $.NSUTF8StringEncoding).js
      : ObjC.unwrap(raw);
    out.workgroups = text;
    JSON.parse(text).forEach(function (g) {
      if (g.uniqueIdentifier !== "builtin.claudeCode") return;
      (g.sessions || []).forEach(function (s) {
        if (s.kind && s.kind.kind === "peer") out.peer = true;
      });
    });
  } catch (e) {}
  var s = $.NSScreen.screens.objectAtIndex(0).frame.size;
  out.screen = [Math.round(s.width), Math.round(s.height)];
  return JSON.stringify(out);
}
