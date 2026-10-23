// Small versions of the libraries Postman's sandbox ships: moment, xml2Json and cheerio.
// They cover what collections commonly use; `pm.js` exposes them through `require`.
(function (global) {
  "use strict";
  function isObj(v) { return v !== null && typeof v === "object"; }
  function pad(n, w) { n = String(Math.abs(Math.trunc(n))); while (n.length < w) n = "0" + n; return n; }

  // ------------------------------------------------------------------ moment

  var UNITS = { y: "year", year: "year", years: "year", M: "month", month: "month", months: "month",
    w: "week", week: "week", weeks: "week", d: "day", day: "day", days: "day", date: "day",
    h: "hour", hour: "hour", hours: "hour", m: "minute", minute: "minute", minutes: "minute",
    s: "second", second: "second", seconds: "second", ms: "millisecond", millisecond: "millisecond", milliseconds: "millisecond" };
  var MS = { millisecond: 1, second: 1e3, minute: 6e4, hour: 36e5, day: 864e5, week: 6048e5 };
  var MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
  var DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

  function Moment(date, utc) { this._d = date; this._utc = !!utc; }
  function get(m, part) { return m._d[(m._utc ? "getUTC" : "get") + part](); }
  function set(m, part, v) { m._d[(m._utc ? "setUTC" : "set") + part](v); return m; }

  function parseWith(str, format, utc) {
    var parts = { YYYY: 0, MM: 1, DD: 1, HH: 0, mm: 0, ss: 0, SSS: 0 };
    var re = /YYYY|MM|DD|HH|mm|ss|SSS/g, tokens = [], m, src = "";
    var last = 0;
    while ((m = re.exec(format))) {
      src += format.slice(last, m.index).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      src += m[0] === "YYYY" ? "(\\d{4})" : m[0] === "SSS" ? "(\\d{3})" : "(\\d{1,2})";
      tokens.push(m[0]);
      last = re.lastIndex;
    }
    src += format.slice(last).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    var found = new RegExp("^" + src).exec(str);
    if (!found) return new Date(NaN);
    tokens.forEach(function (t, i) { parts[t] = parseInt(found[i + 1], 10); });
    var args = [parts.YYYY, parts.MM - 1, parts.DD, parts.HH, parts.mm, parts.ss, parts.SSS];
    return utc ? new Date(Date.UTC.apply(null, args)) : new Date(args[0], args[1], args[2], args[3], args[4], args[5], args[6]);
  }
  function make(input, format, utc) {
    if (input instanceof Moment) return new Moment(new Date(input._d.getTime()), utc === undefined ? input._utc : utc);
    var d;
    if (input === undefined || input === null) d = new Date();
    else if (input instanceof Date) d = new Date(input.getTime());
    else if (typeof input === "number") d = new Date(input);
    else if (typeof input === "string" && format) d = parseWith(input, format, utc);
    else if (Array.isArray(input)) d = utc ? new Date(Date.UTC(input[0], input[1] || 0, input[2] || 1, input[3] || 0, input[4] || 0, input[5] || 0)) : new Date(input[0], input[1] || 0, input[2] || 1, input[3] || 0, input[4] || 0, input[5] || 0);
    else d = new Date(input);
    return new Moment(d, utc);
  }
  function moment(input, format) { return make(input, format, false); }
  moment.utc = function (input, format) { return make(input, format, true); };
  moment.unix = function (s) { return make(s * 1000); };
  moment.isMoment = function (v) { return v instanceof Moment; };
  moment.duration = function (n, unit) {
    var ms = typeof n === "number" ? n * (MS[UNITS[unit || "ms"]] || 1) : 0;
    return {
      asMilliseconds: function () { return ms; }, asSeconds: function () { return ms / 1e3; },
      asMinutes: function () { return ms / 6e4; }, asHours: function () { return ms / 36e5; }, asDays: function () { return ms / 864e5; },
      milliseconds: function () { return ms % 1000; }, seconds: function () { return Math.floor(ms / 1e3) % 60; },
      minutes: function () { return Math.floor(ms / 6e4) % 60; }, hours: function () { return Math.floor(ms / 36e5) % 24; },
      days: function () { return Math.floor(ms / 864e5); }
    };
  };

  var P = Moment.prototype;
  P.isValid = function () { return !isNaN(this._d.getTime()); };
  P.clone = function () { return new Moment(new Date(this._d.getTime()), this._utc); };
  P.utc = function () { this._utc = true; return this; };
  P.local = function () { this._utc = false; return this; };
  P.valueOf = function () { return this._d.getTime(); };
  P.unix = function () { return Math.floor(this._d.getTime() / 1000); };
  P.toDate = function () { return new Date(this._d.getTime()); };
  P.toISOString = function () { return this._d.toISOString(); };
  P.toJSON = P.toISOString;
  P.toString = function () { return this._d.toString(); };
  P.utcOffset = function () { return this._utc ? 0 : -this._d.getTimezoneOffset(); };
  P.daysInMonth = function () { return new Date(get(this, "FullYear"), get(this, "Month") + 1, 0).getDate(); };
  [["year", "FullYear"], ["month", "Month"], ["date", "Date"], ["hour", "Hours"], ["minute", "Minutes"], ["second", "Seconds"], ["millisecond", "Milliseconds"]].forEach(function (p) {
    P[p[0]] = P[p[0] + "s"] = function (v) { return v === undefined ? get(this, p[1]) : set(this, p[1], v); };
  });
  P.day = function () { return get(this, "Day"); };
  P.weekday = P.day;
  function addTo(m, n, unit, sign) {
    if (isObj(n)) { Object.keys(n).forEach(function (u) { addTo(m, n[u], u, sign); }); return m; }
    var u = UNITS[unit] || "millisecond";
    n = Number(n) * sign;
    if (u === "month") set(m, "Month", get(m, "Month") + n);
    else if (u === "year") set(m, "FullYear", get(m, "FullYear") + n);
    else m._d = new Date(m._d.getTime() + n * MS[u]);
    return m;
  }
  P.add = function (n, unit) { return addTo(this, n, unit, 1); };
  P.subtract = function (n, unit) { return addTo(this, n, unit, -1); };
  P.startOf = function (unit) {
    var u = UNITS[unit];
    var order = ["year", "month", "day", "hour", "minute", "second"];
    var idx = order.indexOf(u === "week" ? "day" : u);
    if (idx < 0) return this;
    if (idx < 1) set(this, "Month", 0);
    if (idx < 2) set(this, "Date", 1);
    if (idx < 3) set(this, "Hours", 0);
    if (idx < 4) set(this, "Minutes", 0);
    if (idx < 5) set(this, "Seconds", 0);
    set(this, "Milliseconds", 0);
    if (u === "week") this.subtract(get(this, "Day"), "day");
    return this;
  };
  P.endOf = function (unit) {
    var u = UNITS[unit];
    return this.startOf(unit).add(1, u === "day" ? "day" : unit).subtract(1, "ms");
  };
  P.diff = function (other, unit, float) {
    var a = this, b = make(other), u = UNITS[unit] || "millisecond", r;
    if (u === "month" || u === "year") {
      r = (get(a, "FullYear") - get(b, "FullYear")) * 12 + (get(a, "Month") - get(b, "Month"));
      var anchor = b.clone().add(r, "month");
      r += (a.valueOf() - anchor.valueOf()) / (Math.abs(anchor.clone().add(1, "month").valueOf() - anchor.valueOf()) || 1);
      if (u === "year") r /= 12;
    } else r = (a.valueOf() - b.valueOf()) / MS[u];
    return float ? r : (r < 0 ? Math.ceil(r) : Math.floor(r));
  };
  P.isBefore = function (o, unit) { return unit ? this.clone().endOf(unit).valueOf() < make(o).startOf(unit).valueOf() : this.valueOf() < make(o).valueOf(); };
  P.isAfter = function (o, unit) { return unit ? this.clone().startOf(unit).valueOf() > make(o).endOf(unit).valueOf() : this.valueOf() > make(o).valueOf(); };
  P.isSame = function (o, unit) { return unit ? this.clone().startOf(unit).valueOf() === make(o).startOf(unit).valueOf() : this.valueOf() === make(o).valueOf(); };
  P.isBetween = function (a, b) { var v = this.valueOf(); return v > make(a).valueOf() && v < make(b).valueOf(); };
  P.fromNow = function () {
    var s = (Date.now() - this.valueOf()) / 1000, future = s < 0, a = Math.abs(s), text;
    if (a < 45) text = "a few seconds"; else if (a < 5400) text = Math.round(a / 60) + " minutes";
    else if (a < 129600) text = Math.round(a / 3600) + " hours"; else if (a < 2592000) text = Math.round(a / 86400) + " days";
    else if (a < 31536000) text = Math.round(a / 2592000) + " months"; else text = Math.round(a / 31536000) + " years";
    text = text.replace(/^1 (\w+)s$/, "a $1");
    return future ? "in " + text : text + " ago";
  };
  P.format = function (fmt) {
    if (!this.isValid()) return "Invalid date";
    var m = this;
    if (fmt === undefined) fmt = this._utc ? "YYYY-MM-DDTHH:mm:ss[Z]" : "YYYY-MM-DDTHH:mm:ssZ";
    var off = m.utcOffset(), offH = pad(Math.floor(Math.abs(off) / 60), 2), offM = pad(Math.abs(off) % 60, 2), sign = off < 0 ? "-" : "+";
    var h = get(m, "Hours");
    var map = {
      YYYY: pad(get(m, "FullYear"), 4), YY: pad(get(m, "FullYear") % 100, 2),
      MMMM: MONTHS[get(m, "Month")], MMM: MONTHS[get(m, "Month")].slice(0, 3), MM: pad(get(m, "Month") + 1, 2), M: String(get(m, "Month") + 1),
      DD: pad(get(m, "Date"), 2), D: String(get(m, "Date")),
      dddd: DAYS[get(m, "Day")], ddd: DAYS[get(m, "Day")].slice(0, 3), d: String(get(m, "Day")),
      HH: pad(h, 2), H: String(h), hh: pad(h % 12 || 12, 2), h: String(h % 12 || 12),
      mm: pad(get(m, "Minutes"), 2), m: String(get(m, "Minutes")), ss: pad(get(m, "Seconds"), 2), s: String(get(m, "Seconds")),
      SSS: pad(get(m, "Milliseconds"), 3), A: h < 12 ? "AM" : "PM", a: h < 12 ? "am" : "pm",
      ZZ: sign + offH + offM, Z: sign + offH + ":" + offM, X: String(m.unix()), x: String(m.valueOf())
    };
    return fmt.replace(/\[([^\]]*)]|YYYY|YY|MMMM|MMM|MM|M|DD|D|dddd|ddd|d|HH|H|hh|h|mm|m|ss|s|SSS|A|a|ZZ|Z|X|x/g, function (t, lit) {
      return lit !== undefined ? lit : map[t];
    });
  };

  // ------------------------------------------------------------------ a tolerant markup parser

  var VOID = { area: 1, base: 1, br: 1, col: 1, embed: 1, hr: 1, img: 1, input: 1, link: 1, meta: 1, param: 1, source: 1, track: 1, wbr: 1 };
  var RAW = { script: 1, style: 1 };
  var ENT = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
  function decode(s) {
    return s.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, function (m, e) {
      if (e[0] === "#") { var n = e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10); return isNaN(n) ? m : String.fromCodePoint(n); }
      return ENT[e.toLowerCase()] !== undefined ? ENT[e.toLowerCase()] : m;
    });
  }
  function parseMarkup(src, html) {
    var root = { type: "root", children: [], parent: null };
    var stack = [root], i = 0;
    function top() { return stack[stack.length - 1]; }
    function addText(t) { if (t) top().children.push({ type: "text", data: decode(t), parent: top() }); }
    while (i < src.length) {
      var lt = src.indexOf("<", i);
      if (lt < 0) { addText(src.slice(i)); break; }
      addText(src.slice(i, lt));
      if (src.startsWith("<!--", lt)) { var e = src.indexOf("-->", lt); i = e < 0 ? src.length : e + 3; continue; }
      if (src.startsWith("<![CDATA[", lt)) { var c = src.indexOf("]]>", lt); top().children.push({ type: "text", data: src.slice(lt + 9, c < 0 ? src.length : c), parent: top() }); i = c < 0 ? src.length : c + 3; continue; }
      if (src[lt + 1] === "!" || src[lt + 1] === "?") { var g = src.indexOf(">", lt); i = g < 0 ? src.length : g + 1; continue; }
      var gt = src.indexOf(">", lt);
      if (gt < 0) { addText(src.slice(lt)); break; }
      var tag = src.slice(lt + 1, gt);
      i = gt + 1;
      if (tag[0] === "/") {
        var name = tag.slice(1).trim();
        if (html) name = name.toLowerCase();
        for (var k = stack.length - 1; k > 0; k--) {
          if (stack[k].name === name) { stack.length = k; break; }
        }
        continue;
      }
      var selfClose = /\/\s*$/.test(tag);
      if (selfClose) tag = tag.replace(/\/\s*$/, "");
      var nm = /^([^\s/>]+)/.exec(tag);
      if (!nm) { addText("<" + tag + ">"); continue; }
      var tname = html ? nm[1].toLowerCase() : nm[1];
      var attribs = {};
      var attrRe = /([^\s=]+)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g, am;
      var rest = tag.slice(nm[1].length);
      while ((am = attrRe.exec(rest))) {
        attribs[html ? am[1].toLowerCase() : am[1]] = decode(am[3] !== undefined ? am[3] : am[4] !== undefined ? am[4] : am[5] !== undefined ? am[5] : "");
      }
      var el = { type: "tag", name: tname, attribs: attribs, children: [], parent: top() };
      top().children.push(el);
      if (html && RAW[tname]) {
        var close = src.toLowerCase().indexOf("</" + tname, i);
        var end = close < 0 ? src.length : close;
        if (end > i) el.children.push({ type: "text", data: src.slice(i, end), parent: el });
        var after = close < 0 ? src.length : src.indexOf(">", close);
        i = after < 0 ? src.length : after + 1;
        continue;
      }
      if (!selfClose && !(html && VOID[tname])) stack.push(el);
    }
    return root;
  }
  function textOf(n) {
    if (n.type === "text") return n.data;
    return (n.children || []).map(textOf).join("");
  }
  function esc(s) { return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); }
  function outer(n) {
    if (n.type === "text") return esc(n.data);
    if (n.type === "root") return n.children.map(outer).join("");
    var attrs = Object.keys(n.attribs).map(function (k) { return " " + k + '="' + n.attribs[k].replace(/"/g, "&quot;") + '"'; }).join("");
    if (VOID[n.name]) return "<" + n.name + attrs + ">";
    return "<" + n.name + attrs + ">" + n.children.map(outer).join("") + "</" + n.name + ">";
  }

  // ------------------------------------------------------------------ xml2Json (like xml2js, explicitArray: false)

  function toJson(el) {
    var obj = {}, hasAttr = Object.keys(el.attribs).length > 0;
    if (hasAttr) obj.$ = el.attribs;
    var text = "", hasChild = false;
    el.children.forEach(function (c) {
      if (c.type === "text") { text += c.data; return; }
      hasChild = true;
      var v = toJson(c);
      if (obj[c.name] === undefined) obj[c.name] = v;
      else if (Array.isArray(obj[c.name])) obj[c.name].push(v);
      else obj[c.name] = [obj[c.name], v];
    });
    text = text.trim();
    if (!hasChild && !hasAttr) return text;
    if (text) obj._ = text;
    return obj;
  }
  function xml2Json(xml) {
    var root = parseMarkup(String(xml), false), out = {};
    root.children.forEach(function (c) { if (c.type === "tag") out[c.name] = toJson(c); });
    return out;
  }

  // ------------------------------------------------------------------ cheerio (a jQuery-like subset)

  function parseSelector(sel) {
    return sel.split(",").map(function (group) {
      var steps = [], comb = " ", re = /\s*(>|\+|~)\s*|\s+|([^\s>+~]+)/g, m;
      group = group.trim();
      while ((m = re.exec(group))) {
        if (m[1]) comb = m[1];
        else if (m[2]) { steps.push({ comb: comb, simple: parseSimple(m[2]) }); comb = " "; }
        else comb = " ";
      }
      return steps;
    });
  }
  function parseSimple(s) {
    var out = { tag: null, id: null, classes: [], attrs: [], pseudos: [] };
    var re = /^([a-zA-Z][\w-]*|\*)|#([\w-]+)|\.([\w-]+)|\[([^\]=~^$*|]+)(?:([~^$*|]?=)["']?([^"'\]]*)["']?)?\]|:([\w-]+)(?:\(([^)]*)\))?/g, m;
    while ((m = re.exec(s))) {
      if (m[0] === "") { re.lastIndex++; continue; }
      if (m[1]) out.tag = m[1] === "*" ? null : m[1].toLowerCase();
      else if (m[2]) out.id = m[2];
      else if (m[3]) out.classes.push(m[3]);
      else if (m[4]) out.attrs.push({ name: m[4].trim().toLowerCase(), op: m[5], value: m[6] });
      else if (m[7]) out.pseudos.push({ name: m[7], arg: m[8] });
    }
    return out;
  }
  function siblings(el) { return el.parent ? el.parent.children.filter(function (c) { return c.type === "tag"; }) : [el]; }
  function matchSimple(el, s) {
    if (el.type !== "tag") return false;
    if (s.tag && el.name !== s.tag) return false;
    if (s.id && el.attribs.id !== s.id) return false;
    var cls = (el.attribs["class"] || "").split(/\s+/);
    if (!s.classes.every(function (c) { return cls.indexOf(c) >= 0; })) return false;
    var okAttrs = s.attrs.every(function (a) {
      var v = el.attribs[a.name];
      if (v === undefined) return false;
      switch (a.op) {
        case "=": return v === a.value; case "^=": return v.indexOf(a.value) === 0;
        case "$=": return v.slice(-a.value.length) === a.value; case "*=": return v.indexOf(a.value) >= 0;
        case "~=": return v.split(/\s+/).indexOf(a.value) >= 0; case "|=": return v === a.value || v.indexOf(a.value + "-") === 0;
        default: return true;
      }
    });
    if (!okAttrs) return false;
    return s.pseudos.every(function (p) {
      var sib = siblings(el), idx = sib.indexOf(el);
      if (p.name === "first-child") return idx === 0;
      if (p.name === "last-child") return idx === sib.length - 1;
      if (p.name === "nth-child") { var n = parseInt(p.arg, 10); return p.arg === "odd" ? idx % 2 === 0 : p.arg === "even" ? idx % 2 === 1 : idx === n - 1; }
      if (p.name === "contains") return textOf(el).indexOf((p.arg || "").replace(/^["']|["']$/g, "")) >= 0;
      return true;
    });
  }
  function matches(el, steps) {
    // Right to left: the last step is the element itself.
    function at(node, i) {
      if (!matchSimple(node, steps[i].simple)) return false;
      if (i === 0) return true;
      var comb = steps[i].comb;
      if (comb === ">") return node.parent && node.parent.type === "tag" && at(node.parent, i - 1);
      if (comb === "+" || comb === "~") {
        var sib = siblings(node), idx = sib.indexOf(node);
        if (comb === "+") return idx > 0 && at(sib[idx - 1], i - 1);
        for (var k = idx - 1; k >= 0; k--) if (at(sib[k], i - 1)) return true;
        return false;
      }
      for (var p = node.parent; p && p.type === "tag"; p = p.parent) if (at(p, i - 1)) return true;
      return false;
    }
    return at(el, steps.length - 1);
  }
  function descendants(n, out) {
    (n.children || []).forEach(function (c) { if (c.type === "tag") { out.push(c); descendants(c, out); } });
    return out;
  }
  function select(selector, roots) {
    var groups = parseSelector(selector), out = [];
    roots.forEach(function (r) {
      descendants(r, []).forEach(function (el) {
        if (out.indexOf(el) < 0 && groups.some(function (g) { return g.length && matches(el, g); })) out.push(el);
      });
    });
    return out;
  }

  function Selection(nodes, root) {
    this._nodes = nodes; this._root = root; this.length = nodes.length;
    for (var i = 0; i < nodes.length; i++) this[i] = nodes[i];
  }
  var S = Selection.prototype;
  S._wrap = function (nodes) { return new Selection(nodes, this._root); };
  S.each = function (fn) { for (var i = 0; i < this._nodes.length; i++) if (fn.call(this._nodes[i], i, this._nodes[i]) === false) break; return this; };
  S.map = function (fn) {
    var out = [];
    this._nodes.forEach(function (n, i) { var v = fn.call(n, i, n); if (v !== null && v !== undefined) out = out.concat(v); });
    var arr = out.slice();
    arr.get = function () { return out.slice(); };
    arr.toArray = arr.get;
    return arr;
  };
  S.get = function (i) { return i === undefined ? this._nodes.slice() : this._nodes[i < 0 ? this._nodes.length + i : i]; };
  S.toArray = function () { return this._nodes.slice(); };
  S.eq = function (i) { var n = this.get(i); return this._wrap(n ? [n] : []); };
  S.first = function () { return this.eq(0); };
  S.last = function () { return this.eq(-1); };
  S.find = function (sel) { return this._wrap(select(sel, this._nodes)); };
  S.children = function (sel) {
    var out = [];
    this._nodes.forEach(function (n) { (n.children || []).forEach(function (c) { if (c.type === "tag" && (!sel || select(sel, [{ children: [c] }]).indexOf(c) >= 0)) out.push(c); }); });
    return this._wrap(out);
  };
  S.parent = function () {
    var out = [];
    this._nodes.forEach(function (n) { if (n.parent && n.parent.type === "tag" && out.indexOf(n.parent) < 0) out.push(n.parent); });
    return this._wrap(out);
  };
  S.next = function () {
    var out = [];
    this._nodes.forEach(function (n) { var sib = siblings(n), i = sib.indexOf(n); if (sib[i + 1]) out.push(sib[i + 1]); });
    return this._wrap(out);
  };
  S.prev = function () {
    var out = [];
    this._nodes.forEach(function (n) { var sib = siblings(n), i = sib.indexOf(n); if (i > 0) out.push(sib[i - 1]); });
    return this._wrap(out);
  };
  S.filter = function (f) {
    var self = this;
    return this._wrap(this._nodes.filter(function (n, i) {
      return typeof f === "function" ? f.call(n, i, n) : parseSelector(f).some(function (g) { return matches(n, g); });
    }));
  };
  S.is = function (sel) { return this.filter(sel).length > 0; };
  S.text = function () { return this._nodes.map(textOf).join(""); };
  S.html = function () { var n = this._nodes[0]; return n ? (n.children || []).map(outer).join("") : null; };
  S.attr = function (name) { var n = this._nodes[0]; return n && n.attribs ? n.attribs[name] : undefined; };
  S.data = function (name) { return this.attr("data-" + name); };
  S.val = function () { var n = this._nodes[0]; if (!n) return undefined; return n.name === "textarea" ? textOf(n) : n.attribs.value; };
  S.hasClass = function (c) { return this._nodes.some(function (n) { return (n.attribs && n.attribs["class"] || "").split(/\s+/).indexOf(c) >= 0; }); };
  S.toString = function () { return this._nodes.map(outer).join(""); };

  var cheerio = {
    load: function (html) {
      var root = parseMarkup(String(html), true);
      var $ = function (sel, ctx) {
        if (sel instanceof Selection) return sel;
        if (isObj(sel) && sel.type) return new Selection([sel], root);
        if (Array.isArray(sel)) return new Selection(sel, root);
        var roots = ctx instanceof Selection ? ctx._nodes : isObj(ctx) && ctx.type ? [ctx] : [root];
        return new Selection(select(String(sel), roots), root);
      };
      $.root = function () { return new Selection([root], root); };
      $.html = function (s) { return s ? s.toString() : outer(root); };
      $.text = function () { return textOf(root); };
      return $;
    }
  };

  // ------------------------------------------------------------------ Handlebars (for pm.visualizer)

  function hbsEsc(v) {
    return String(v === undefined || v === null ? "" : v).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
  function lookup(path, stack) {
    path = path.trim();
    if (path === "this" || path === ".") return stack[stack.length - 1].ctx;
    var frame = stack[stack.length - 1];
    if (path[0] === "@") return frame.meta[path.slice(1)];
    var ups = 0;
    while (path.indexOf("../") === 0) { ups++; path = path.slice(3); }
    var base = stack[Math.max(0, stack.length - 1 - ups)].ctx;
    var parts = path.replace(/^this\./, "").split(".");
    var v = base;
    for (var i = 0; i < parts.length; i++) { if (v === null || v === undefined) return undefined; v = v[parts[i]]; }
    if (v === undefined && ups === 0 && stack.length > 1 && parts.length === 1) {
      // Like Handlebars, fall back to outer contexts for a bare name.
      for (var k = stack.length - 2; k >= 0; k--) { var o = stack[k].ctx; if (isObj(o) && parts[0] in o) return o[parts[0]]; }
    }
    return v;
  }
  function truthy(v) { return Array.isArray(v) ? v.length > 0 : !!v; }
  // Splits a template into text, {{tags}} and {{#blocks}}...{{/blocks}}.
  function hbsParse(src) {
    var re = /\{\{(\{?)\s*([#\/^]?)([^}]*?)\s*\}?\}\}/g, root = { children: [] }, stack = [root], last = 0, m;
    while ((m = re.exec(src))) {
      if (m.index > last) stack[stack.length - 1].children.push({ text: src.slice(last, m.index) });
      last = re.lastIndex;
      var raw = m[1] === "{", kind = m[2], body = m[3].trim();
      if (body.indexOf("!") === 0) continue;
      if (kind === "#") {
        var sp = body.indexOf(" "), node = { block: sp < 0 ? body : body.slice(0, sp), arg: sp < 0 ? "" : body.slice(sp + 1), children: [], inverse: [] };
        stack[stack.length - 1].children.push(node);
        stack.push(node);
      } else if (kind === "/") {
        if (stack.length > 1) stack.pop();
      } else if (body === "else" && stack.length > 1) {
        var b = stack[stack.length - 1];
        b.main = b.children; b.children = b.inverse;
      } else {
        stack[stack.length - 1].children.push({ tag: body, raw: raw });
      }
    }
    if (last < src.length) root.children.push({ text: src.slice(last) });
    (function fix(n) { (n.children || []).forEach(fix); if (n.main) { n.inverse = n.children; n.children = n.main; } })(root);
    return root.children;
  }
  function hbsRender(nodes, stack) {
    return nodes.map(function (n) {
      if (n.text !== undefined) return n.text;
      if (n.tag !== undefined) { var v = lookup(n.tag, stack); return n.raw ? (v === undefined ? "" : String(isObj(v) ? JSON.stringify(v) : v)) : hbsEsc(isObj(v) ? JSON.stringify(v) : v); }
      var val = lookup(n.arg || "this", stack);
      switch (n.block) {
        case "each":
          var items = Array.isArray(val) ? val : isObj(val) ? Object.keys(val).map(function (k) { return { k: k, v: val[k] }; }) : [];
          if (!items.length) return hbsRender(n.inverse, stack);
          return items.map(function (it, i) {
            var isPair = !Array.isArray(val);
            var ctx = isPair ? it.v : it;
            return hbsRender(n.children, stack.concat([{ ctx: ctx, meta: { index: i, key: isPair ? it.k : i, first: i === 0, last: i === items.length - 1 } }]));
          }).join("");
        case "if": return hbsRender(truthy(val) ? n.children : n.inverse, stack);
        case "unless": return hbsRender(!truthy(val) ? n.children : n.inverse, stack);
        case "with": return truthy(val) ? hbsRender(n.children, stack.concat([{ ctx: val, meta: {} }])) : hbsRender(n.inverse, stack);
        default: return hbsRender(n.children, stack);
      }
    }).join("");
  }
  function handlebars(template, data) {
    return hbsRender(hbsParse(String(template)), [{ ctx: data === undefined ? {} : data, meta: {} }]);
  }

  global.__libs = { moment: moment, xml2Json: xml2Json, cheerio: cheerio, handlebars: handlebars };
})(globalThis);
