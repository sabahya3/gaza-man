// The Postman sandbox API (`pm.*`) for Gaza Man, on top of QuickJS.
//
// Rust sets `__input` (variables, request, response) before this runs, and reads
// `__finish()` afterwards. The only native helpers are `__sendRequest` and `__digest`.
(function (global) {
  "use strict";
  var input = global.__input;
  var out = { changes: [], tests: [], logs: [], errors: [], skip: false, nextRequest: undefined };
  var current = ""; // label of the script that is running, for logs and errors

  // ------------------------------------------------------------------ helpers

  function isObj(v) { return v !== null && typeof v === "object"; }
  function toStr(v) {
    if (v === undefined || v === null) return "";
    if (typeof v === "string") return v;
    if (isObj(v)) { try { return JSON.stringify(v); } catch (e) { return String(v); } }
    return String(v);
  }
  function fmt(v) {
    if (typeof v === "string") return "'" + (v.length > 80 ? v.slice(0, 80) + "…" : v) + "'";
    if (typeof v === "function") return "[Function]";
    if (v === undefined) return "undefined";
    if (isObj(v)) {
      var s;
      try { s = JSON.stringify(v); } catch (e) { s = String(v); }
      return s && s.length > 80 ? s.slice(0, 80) + "…" : s;
    }
    return String(v);
  }
  function logText(args) {
    return Array.prototype.map.call(args, function (a) {
      if (typeof a === "string") return a;
      if (a instanceof Error) return a.name + ": " + a.message;
      if (isObj(a)) { try { return JSON.stringify(a, null, 2); } catch (e) { return String(a); } }
      return String(a);
    }).join(" ");
  }
  function errText(e) {
    if (e && e.name && e.message !== undefined) return e.name + ": " + e.message;
    return String(e);
  }
  function deepEqual(a, b) {
    if (a === b) return true;
    if (typeof a === "number" && typeof b === "number" && isNaN(a) && isNaN(b)) return true;
    if (!isObj(a) || !isObj(b)) return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    var ka = Object.keys(a), kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    for (var i = 0; i < ka.length; i++) {
      if (!Object.prototype.hasOwnProperty.call(b, ka[i]) || !deepEqual(a[ka[i]], b[ka[i]])) return false;
    }
    return true;
  }
  function own(o, k) { return Object.prototype.hasOwnProperty.call(o, k); }

  // UTF-8 bytes <-> hex, for the crypto helpers.
  function utf8Hex(s) {
    s = toStr(s);
    var hex = "";
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length) {
        c = 0x10000 + ((c - 0xd800) << 10) + (s.charCodeAt(++i) - 0xdc00);
      }
      var bytes = c < 0x80 ? [c]
        : c < 0x800 ? [0xc0 | (c >> 6), 0x80 | (c & 63)]
        : c < 0x10000 ? [0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63)]
        : [0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63)];
      for (var j = 0; j < bytes.length; j++) hex += (bytes[j] < 16 ? "0" : "") + bytes[j].toString(16);
    }
    return hex;
  }
  function hexUtf8(hex) {
    var bytes = [];
    for (var i = 0; i + 1 < hex.length; i += 2) bytes.push(parseInt(hex.substr(i, 2), 16));
    var s = "", i2 = 0;
    while (i2 < bytes.length) {
      var b = bytes[i2++], c;
      if (b < 0x80) c = b;
      else if (b < 0xe0) c = ((b & 31) << 6) | (bytes[i2++] & 63);
      else if (b < 0xf0) c = ((b & 15) << 12) | ((bytes[i2++] & 63) << 6) | (bytes[i2++] & 63);
      else c = ((b & 7) << 18) | ((bytes[i2++] & 63) << 12) | ((bytes[i2++] & 63) << 6) | (bytes[i2++] & 63);
      if (c > 0xffff) { c -= 0x10000; s += String.fromCharCode(0xd800 + (c >> 10), 0xdc00 + (c & 1023)); }
      else s += String.fromCharCode(c);
    }
    return s;
  }
  var B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function hexB64(hex) {
    var out = "";
    for (var i = 0; i < hex.length; i += 6) {
      var chunk = hex.substr(i, 6), n = parseInt((chunk + "000000").slice(0, 6), 16), len = chunk.length / 2;
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (len > 1 ? B64[(n >> 6) & 63] : "=") + (len > 2 ? B64[n & 63] : "=");
    }
    return out;
  }
  function b64Hex(s) {
    s = toStr(s).replace(/[^A-Za-z0-9+/]/g, "");
    var hex = "", bits = 0, acc = 0;
    for (var i = 0; i < s.length; i++) {
      acc = (acc << 6) | B64.indexOf(s[i]); bits += 6;
      if (bits >= 8) { bits -= 8; var b = (acc >> bits) & 255; hex += (b < 16 ? "0" : "") + b.toString(16); }
    }
    return hex;
  }
  // btoa / atob work on "binary strings" (one char per byte), as in browsers.
  global.btoa = function (s) {
    s = toStr(s); var hex = "";
    for (var i = 0; i < s.length; i++) { var c = s.charCodeAt(i) & 255; hex += (c < 16 ? "0" : "") + c.toString(16); }
    return hexB64(hex);
  };
  global.atob = function (s) {
    var hex = b64Hex(s), out = "";
    for (var i = 0; i < hex.length; i += 2) out += String.fromCharCode(parseInt(hex.substr(i, 2), 16));
    return out;
  };

  // ------------------------------------------------------------------ variables

  var dynamic = {
    $guid: uuid4, $randomUUID: uuid4,
    $timestamp: function () { return String(Math.floor(Date.now() / 1000)); },
    $isoTimestamp: function () { return new Date().toISOString(); },
    $randomInt: function () { return String(Math.floor(Math.random() * 1001)); }
  };
  function uuid4() {
    var h = "0123456789abcdef", s = "";
    for (var i = 0; i < 36; i++) {
      if (i === 8 || i === 13 || i === 18 || i === 23) s += "-";
      else if (i === 14) s += "4";
      else if (i === 19) s += h[(Math.random() * 4 | 0) + 8];
      else s += h[Math.random() * 16 | 0];
    }
    return s;
  }

  var vars = input.vars;
  var locals = vars.locals || {};

  function Scope(name, store, readOnly) {
    this._name = name; this._store = store; this._ro = readOnly;
    if (name === "environment") this.name = input.envName || undefined;
  }
  Scope.prototype.get = function (k) { return own(this._store, k) ? this._store[k] : undefined; };
  Scope.prototype.has = function (k) { return own(this._store, k); };
  Scope.prototype.toObject = function () { return Object.assign({}, this._store); };
  Scope.prototype.toJSON = Scope.prototype.toObject;
  Scope.prototype.replaceIn = function (t) { return replaceIn(t); };
  Scope.prototype.set = function (k, v) {
    if (this._ro) throw new Error("pm." + this._name + " is read-only");
    k = toStr(k); v = toStr(v);
    this._store[k] = v;
    out.changes.push({ op: "set", scope: this._name, key: k, value: v });
  };
  Scope.prototype.unset = function (k) {
    if (this._ro) throw new Error("pm." + this._name + " is read-only");
    k = toStr(k);
    delete this._store[k];
    out.changes.push({ op: "unset", scope: this._name, key: k });
  };
  Scope.prototype.clear = function () {
    if (this._ro) throw new Error("pm." + this._name + " is read-only");
    for (var k in this._store) delete this._store[k];
    out.changes.push({ op: "clear", scope: this._name });
  };

  var vaultStore = vars.vault || {};
  var globalsScope = new Scope("globals", vars.globals || {});
  var collectionScope = new Scope("collection", vars.collection || {});
  var environmentScope = new Scope("environment", vars.environment || {});
  var dataScope = new Scope("iterationData", vars.data || {}, true);

  // Postman's resolution order: local, iteration data, environment, collection, globals.
  var layers = [locals, dataScope._store, environmentScope._store, collectionScope._store, globalsScope._store];
  function lookup(k) {
    for (var i = 0; i < layers.length; i++) if (own(layers[i], k)) return layers[i][k];
    return undefined;
  }
  function replaceIn(t) {
    if (typeof t !== "string") return t;
    for (var round = 0; round < 5 && t.indexOf("{{") >= 0; round++) {
      t = t.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, function (m, name) {
        if (dynamic[name]) return dynamic[name]();
        var v = lookup(name);
        return v === undefined ? m : v;
      });
    }
    return t;
  }
  var variables = {
    get: function (k) { return lookup(toStr(k)); },
    has: function (k) { return lookup(toStr(k)) !== undefined; },
    set: function (k, v) { locals[toStr(k)] = toStr(v); },
    unset: function (k) { delete locals[toStr(k)]; },
    toObject: function () {
      var o = {};
      for (var i = layers.length - 1; i >= 0; i--) Object.assign(o, layers[i]);
      return o;
    },
    replaceIn: replaceIn
  };

  // ------------------------------------------------------------------ lists (headers, query)

  function PropertyList(items) { this._items = items || []; }
  function keyOf(item) { return toStr(item.key).toLowerCase(); }
  function toItem(a, b) {
    if (typeof a === "string" && b === undefined && a.indexOf(":") > 0) {
      var i = a.indexOf(":");
      return { key: a.slice(0, i).trim(), value: a.slice(i + 1).trim() };
    }
    if (typeof a === "string") return { key: a, value: toStr(b) };
    return { key: toStr(a.key), value: toStr(a.value), disabled: !!a.disabled };
  }
  PropertyList.prototype.get = function (k) {
    k = toStr(k).toLowerCase();
    for (var i = this._items.length - 1; i >= 0; i--) {
      var it = this._items[i];
      if (!it.disabled && keyOf(it) === k) return it.value;
    }
    return undefined;
  };
  PropertyList.prototype.one = function (k) {
    k = toStr(k).toLowerCase();
    return this._items.find(function (it) { return !it.disabled && keyOf(it) === k; });
  };
  PropertyList.prototype.has = function (k, v) {
    var got = this.get(k);
    return v === undefined ? got !== undefined : got === toStr(v);
  };
  PropertyList.prototype.add = function (a, b) { this._items.push(toItem(a, b)); };
  PropertyList.prototype.append = PropertyList.prototype.add;
  PropertyList.prototype.upsert = function (a, b) {
    var item = toItem(a, b), k = keyOf(item);
    var found = this._items.find(function (it) { return keyOf(it) === k; });
    if (found) { found.value = item.value; found.disabled = false; } else this._items.push(item);
  };
  PropertyList.prototype.remove = function (k) {
    var pred = typeof k === "function" ? k : function (it) { return keyOf(it) === toStr(isObj(k) ? k.key : k).toLowerCase(); };
    this._items = this._items.filter(function (it) { return !pred(it); });
  };
  PropertyList.prototype.clear = function () { this._items = []; };
  PropertyList.prototype.all = function () { return this._items.filter(function (it) { return !it.disabled; }); };
  PropertyList.prototype.count = function () { return this.all().length; };
  PropertyList.prototype.each = function (fn) { this.all().forEach(fn); };
  PropertyList.prototype.map = function (fn) { return this.all().map(fn); };
  PropertyList.prototype.filter = function (fn) { return this.all().filter(fn); };
  PropertyList.prototype.find = function (fn) { return this.all().find(fn); };
  PropertyList.prototype.toObject = function () {
    var o = {};
    this.all().forEach(function (it) { o[it.key] = it.value; });
    return o;
  };
  PropertyList.prototype.toJSON = function () { return this._items; };

  // ------------------------------------------------------------------ the request

  function Url(raw) { this._set(raw); }
  Url.prototype._set = function (raw) {
    raw = toStr(raw);
    var q = raw.indexOf("?");
    this._base = q >= 0 ? raw.slice(0, q) : raw;
    var pairs = q >= 0 ? raw.slice(q + 1).split("&").filter(function (p) { return p !== ""; }) : [];
    this.query = new PropertyList(pairs.map(function (p) {
      var i = p.indexOf("=");
      return i >= 0 ? { key: p.slice(0, i), value: p.slice(i + 1) } : { key: p, value: "" };
    }));
  };
  Url.prototype.toString = function () {
    var qs = this.query._items.filter(function (it) { return !it.disabled; })
      .map(function (it) { return it.value === "" ? it.key : it.key + "=" + it.value; }).join("&");
    return this._base + (qs ? "?" + qs : "");
  };
  Url.prototype.getQueryString = function () { var s = this.toString(); var i = s.indexOf("?"); return i >= 0 ? s.slice(i + 1) : ""; };
  Url.prototype.getHost = function () { return this._base.replace(/^[a-z]+:\/\//i, "").split(/[/:]/)[0]; };
  Url.prototype.getPath = function () {
    var rest = this._base.replace(/^[a-z]+:\/\//i, "");
    var i = rest.indexOf("/");
    return i >= 0 ? rest.slice(i) : "/";
  };
  Url.prototype.update = function (raw) { this._set(raw); };
  Url.prototype.toJSON = function () { return this.toString(); };
  Object.defineProperty(Url.prototype, "host", { get: function () { return this.getHost().split("."); } });
  Object.defineProperty(Url.prototype, "path", { get: function () { return this.getPath().split("/").filter(Boolean); } });

  function makeRequest(r) {
    var req = {
      id: r.id, name: r.name, method: r.method,
      headers: new PropertyList(r.headers),
      body: r.body || { mode: "none" },
      auth: r.auth || null
    };
    var url = new Url(r.url);
    Object.defineProperty(req, "url", {
      enumerable: true,
      get: function () { return url; },
      set: function (v) { url = v instanceof Url ? v : new Url(isObj(v) && v.raw ? v.raw : v); }
    });
    req.addHeader = function (h) { req.headers.add(h); };
    req.removeHeader = function (k) { req.headers.remove(k); };
    if (req.body && req.body.mode === "raw") {
      req.body.toString = function () { return toStr(req.body.raw); };
    }
    return req;
  }
  var request = makeRequest(input.request);

  // ------------------------------------------------------------------ expect (a Chai subset)

  function AssertionError(message) { this.name = "AssertionError"; this.message = message; }
  AssertionError.prototype = Object.create(Error.prototype);
  AssertionError.prototype.constructor = AssertionError;

  function Assertion(obj, flags) {
    this._obj = obj;
    this._not = !!(flags && flags.not);
    this._deep = !!(flags && flags.deep);
  }
  Assertion.prototype._assert = function (ok, msg, notMsg) {
    if (this._not ? ok : !ok) throw new AssertionError(this._not ? notMsg : msg);
    return this;
  };
  var chains = ["to", "be", "been", "is", "that", "which", "and", "has", "have", "with", "at", "of", "same", "but", "does", "still", "also", "all", "any", "own"];
  chains.forEach(function (c) { Object.defineProperty(Assertion.prototype, c, { configurable: true, get: function () { return this; } }); });
  Object.defineProperty(Assertion.prototype, "not", { configurable: true, get: function () { this._not = !this._not; return this; } });
  Object.defineProperty(Assertion.prototype, "deep", { configurable: true, get: function () { this._deep = true; return this; } });
  Object.defineProperty(Assertion.prototype, "nested", { configurable: true, get: function () { this._nested = true; return this; } });

  function getter(name, test, what) {
    Object.defineProperty(Assertion.prototype, name, { configurable: true,
      get: function () {
        var o = this._obj;
        return this._assert(test(o), "expected " + fmt(o) + " to be " + what, "expected " + fmt(o) + " not to be " + what);
      }
    });
  }
  getter("ok", function (o) { return !!o; }, "truthy");
  getter("true", function (o) { return o === true; }, "true");
  getter("false", function (o) { return o === false; }, "false");
  getter("null", function (o) { return o === null; }, "null");
  getter("undefined", function (o) { return o === undefined; }, "undefined");
  getter("NaN", function (o) { return typeof o === "number" && isNaN(o); }, "NaN");
  getter("exist", function (o) { return o !== null && o !== undefined; }, "existing");
  getter("empty", function (o) {
    if (typeof o === "string" || Array.isArray(o)) return o.length === 0;
    if (isObj(o)) return Object.keys(o).length === 0;
    return false;
  }, "empty");

  function equalFn(v) {
    var o = this._obj, ok = this._deep ? deepEqual(o, v) : o === v;
    return this._assert(ok, "expected " + fmt(o) + " to " + (this._deep ? "deeply " : "") + "equal " + fmt(v),
      "expected " + fmt(o) + " to not " + (this._deep ? "deeply " : "") + "equal " + fmt(v));
  }
  Assertion.prototype.equal = Assertion.prototype.equals = Assertion.prototype.eq = equalFn;
  Assertion.prototype.eql = Assertion.prototype.eqls = function (v) { this._deep = true; return equalFn.call(this, v); };
  function cmp(name, test, word) {
    Assertion.prototype[name] = function (n) {
      var o = this._obj;
      return this._assert(test(o, n), "expected " + fmt(o) + " to be " + word + " " + fmt(n), "expected " + fmt(o) + " to not be " + word + " " + fmt(n));
    };
  }
  cmp("above", function (o, n) { return o > n; }, "above"); Assertion.prototype.gt = Assertion.prototype.greaterThan = Assertion.prototype.above;
  cmp("below", function (o, n) { return o < n; }, "below"); Assertion.prototype.lt = Assertion.prototype.lessThan = Assertion.prototype.below;
  cmp("least", function (o, n) { return o >= n; }, "at least"); Assertion.prototype.gte = Assertion.prototype.greaterThanOrEqual = Assertion.prototype.least;
  cmp("most", function (o, n) { return o <= n; }, "at most"); Assertion.prototype.lte = Assertion.prototype.lessThanOrEqual = Assertion.prototype.most;
  Assertion.prototype.within = function (a, b) {
    var o = this._obj;
    return this._assert(o >= a && o <= b, "expected " + fmt(o) + " to be within " + a + ".." + b, "expected " + fmt(o) + " to not be within " + a + ".." + b);
  };
  function includeFn(v) {
    var o = this._obj, ok;
    if (typeof o === "string") ok = o.indexOf(toStr(v)) >= 0;
    else if (Array.isArray(o)) ok = o.some(function (x) { return deepEqual(x, v); });
    else if (isObj(o) && isObj(v)) ok = Object.keys(v).every(function (k) { return deepEqual(o[k], v[k]); });
    else ok = false;
    return this._assert(ok, "expected " + fmt(o) + " to include " + fmt(v), "expected " + fmt(o) + " to not include " + fmt(v));
  }
  Assertion.prototype.include = Assertion.prototype.includes = Assertion.prototype.contain = Assertion.prototype.contains = includeFn;
  Assertion.prototype.string = function (s) { return includeFn.call(this, s); };
  Assertion.prototype.property = function (name, value) {
    var o = this._obj, has, got;
    if (this._nested) {
      got = o;
      has = toStr(name).split(".").every(function (p) {
        if (!isObj(got) || !(p in got)) return false;
        got = got[p]; return true;
      });
    } else {
      has = isObj(o) && name in o; got = has ? o[name] : undefined;
    }
    if (arguments.length > 1) {
      var ok = has && (this._deep ? deepEqual(got, value) : got === value);
      this._assert(ok, "expected " + fmt(o) + " to have property '" + name + "' of " + fmt(value) + ", but got " + fmt(got),
        "expected " + fmt(o) + " to not have property '" + name + "' of " + fmt(value));
    } else {
      this._assert(has, "expected " + fmt(o) + " to have property '" + name + "'", "expected " + fmt(o) + " to not have property '" + name + "'");
    }
    // Like Chai, the rest of the chain is about the property's value.
    return new Assertion(got, {});
  };
  Assertion.prototype.lengthOf = function (n) {
    var o = this._obj, len = o === null || o === undefined ? undefined : o.length;
    return this._assert(len === n, "expected " + fmt(o) + " to have a length of " + n + " but got " + len, "expected " + fmt(o) + " to not have a length of " + n);
  };
  Assertion.prototype.length = Assertion.prototype.lengthOf;
  Assertion.prototype.a = Assertion.prototype.an = function (type) {
    var o = this._obj;
    var actual = o === null ? "null" : Array.isArray(o) ? "array" : typeof o;
    type = toStr(type).toLowerCase();
    return this._assert(actual === type, "expected " + fmt(o) + " to be a" + (/^[aeiou]/.test(type) ? "n " : " ") + type, "expected " + fmt(o) + " not to be a" + (/^[aeiou]/.test(type) ? "n " : " ") + type);
  };
  Assertion.prototype.instanceOf = Assertion.prototype.instanceof = function (C) {
    return this._assert(this._obj instanceof C, "expected " + fmt(this._obj) + " to be an instance of " + (C && C.name), "expected " + fmt(this._obj) + " to not be an instance of " + (C && C.name));
  };
  Assertion.prototype.match = Assertion.prototype.matches = function (re) {
    return this._assert(re.test(toStr(this._obj)), "expected " + fmt(this._obj) + " to match " + re, "expected " + fmt(this._obj) + " not to match " + re);
  };
  Assertion.prototype.oneOf = function (list) {
    var o = this._obj;
    return this._assert(list.some(function (x) { return deepEqual(x, o); }), "expected " + fmt(o) + " to be one of " + fmt(list), "expected " + fmt(o) + " to not be one of " + fmt(list));
  };
  Assertion.prototype.keys = Assertion.prototype.key = function () {
    var want = Array.isArray(arguments[0]) ? arguments[0] : Array.prototype.slice.call(arguments);
    var o = this._obj, keys = isObj(o) ? Object.keys(o) : [];
    var ok = want.every(function (k) { return keys.indexOf(k) >= 0; });
    return this._assert(ok, "expected " + fmt(o) + " to have keys " + fmt(want), "expected " + fmt(o) + " to not have keys " + fmt(want));
  };
  Assertion.prototype.members = function (list) {
    var o = this._obj;
    var ok = Array.isArray(o) && list.every(function (x) { return o.some(function (y) { return deepEqual(x, y); }); });
    return this._assert(ok, "expected " + fmt(o) + " to have members " + fmt(list), "expected " + fmt(o) + " to not have members " + fmt(list));
  };
  Assertion.prototype.satisfy = function (fn) {
    return this._assert(!!fn(this._obj), "expected " + fmt(this._obj) + " to satisfy the function", "expected " + fmt(this._obj) + " to not satisfy the function");
  };
  // pm.response.to.have.status(…) and friends, when the subject is a response.
  Assertion.prototype.status = function (s) {
    var r = this._obj, ok = typeof s === "number" ? r.code === s : r.status === s;
    return this._assert(ok, "expected response to have status " + (typeof s === "number" ? "code " : "reason ") + fmt(s) + " but got " + fmt(typeof s === "number" ? r.code : r.status),
      "expected response to not have status " + fmt(s));
  };
  Assertion.prototype.header = function (k, v) {
    var r = this._obj, got = r.headers.get(k);
    var ok = v === undefined ? got !== undefined : got === toStr(v);
    return this._assert(ok, "expected response to have header " + fmt(k) + (v === undefined ? "" : " with value " + fmt(v)), "expected response to not have header " + fmt(k));
  };
  Assertion.prototype.body = function (v) {
    var r = this._obj, text = r.text();
    var ok = v === undefined ? text.length > 0 : v instanceof RegExp ? v.test(text) : isObj(v) ? deepEqual(r.json(), v) : text === toStr(v);
    return this._assert(ok, "expected response body to " + (v === undefined ? "not be empty" : "match " + fmt(v)), "expected response body to not match " + fmt(v));
  };
  Assertion.prototype.jsonBody = function (path, value) {
    var json;
    try { json = this._obj.json(); } catch (e) { return this._assert(false, "expected response body to be JSON", "expected response body not to be JSON"); }
    if (path === undefined) return this._assert(true, "", "expected response body not to be JSON");
    var a = new Assertion(json, { not: this._not });
    a._nested = true;
    return arguments.length > 1 ? a.property(path, value) : a.property(path);
  };
  function statusGetter(name, test, what) {
    Object.defineProperty(Assertion.prototype, name, { configurable: true,
      get: function () {
        var r = this._obj, code = r && r.code;
        return this._assert(test(code), "expected response code to be " + what + " but got " + code, "expected response code not to be " + what);
      }
    });
  }
  statusGetter("success", function (c) { return c >= 200 && c < 300; }, "2XX");
  statusGetter("info", function (c) { return c >= 100 && c < 200; }, "1XX");
  statusGetter("redirection", function (c) { return c >= 300 && c < 400; }, "3XX");
  statusGetter("clientError", function (c) { return c >= 400 && c < 500; }, "4XX");
  statusGetter("serverError", function (c) { return c >= 500; }, "5XX");
  statusGetter("error", function (c) { return c >= 400; }, "4XX or 5XX");
  statusGetter("accepted", function (c) { return c === 202; }, "202");
  statusGetter("badRequest", function (c) { return c === 400; }, "400");
  statusGetter("unauthorized", function (c) { return c === 401; }, "401");
  statusGetter("forbidden", function (c) { return c === 403; }, "403");
  statusGetter("notFound", function (c) { return c === 404; }, "404");
  Object.defineProperty(Assertion.prototype, "json", { configurable: true,
    get: function () {
      var r = this._obj, ok = true;
      try { r.json(); } catch (e) { ok = false; }
      return this._assert(ok, "expected response body to be valid JSON", "expected response body not to be JSON");
    }
  });
  // `ok` on a response means 2XX, like Postman.
  var plainOk = Object.getOwnPropertyDescriptor(Assertion.prototype, "ok").get;
  Object.defineProperty(Assertion.prototype, "ok", { configurable: true,
    get: function () {
      var r = this._obj;
      if (r && r.__isResponse) return this._assert(r.code >= 200 && r.code < 300, "expected response code to be 2XX but got " + r.code, "expected response code not to be 2XX");
      return plainOk.call(this);
    }
  });

  function expect(v, message) {
    var a = new Assertion(v, {});
    if (message) {
      // Prefix the failure with the custom message, like Chai.
      var orig = a._assert;
      a._assert = function (ok, msg, notMsg) {
        try { return orig.call(this, ok, msg, notMsg); } catch (e) { e.message = message + ": " + e.message; throw e; }
      };
    }
    return a;
  }
  expect.fail = function (msg) { throw new AssertionError(msg || "expect.fail()"); };

  // ------------------------------------------------------------------ responses

  function makeResponse(r) {
    var res = {
      __isResponse: true,
      code: r.code, status: r.status, responseTime: r.responseTime, responseSize: r.responseSize,
      headers: new PropertyList(r.headers),
      text: function () { return r.body; },
      json: function () { return JSON.parse(r.body); },
      reason: function () { return r.status; },
      size: function () { return { body: r.responseSize, header: 0, total: r.responseSize }; },
      dataURI: function () { return "data:" + (res.headers.get("content-type") || "text/plain") + ";base64," + global.btoa(unescape(encodeURIComponent(r.body))); }
    };
    res.cookies = new PropertyList((r.cookies || []).map(function (c) { return { key: c.name, value: c.value }; }));
    Object.defineProperty(res, "to", { get: function () { return new Assertion(res, {}); } });
    return res;
  }
  var response = input.response ? makeResponse(input.response) : undefined;

  // ------------------------------------------------------------------ pm.sendRequest

  function normalizeRequest(req) {
    if (typeof req === "string") req = { url: req };
    var headers = [];
    var h = req.header || req.headers;
    if (Array.isArray(h)) h.forEach(function (x) { headers.push(toItem(x)); });
    else if (h instanceof PropertyList) headers = h._items.slice();
    else if (isObj(h)) Object.keys(h).forEach(function (k) { headers.push({ key: k, value: toStr(h[k]) }); });
    var url = req.url instanceof Url ? req.url.toString() : isObj(req.url) ? req.url.raw : req.url;
    return { url: replaceIn(toStr(url)), method: (req.method || "GET").toUpperCase(), headers: headers, body: req.body || null };
  }
  function sendRequest(req, cb) {
    var r = JSON.parse(global.__sendRequest(JSON.stringify(normalizeRequest(req))));
    var err = r.error ? new Error(r.error) : null;
    var res = err ? null : makeResponse(r);
    if (typeof cb === "function") {
      try { cb(err, res); } catch (e) { out.errors.push({ script: current, message: "In the pm.sendRequest callback: " + errText(e) }); }
      return;
    }
    return err ? Promise.reject(err) : Promise.resolve(res);
  }

  // ------------------------------------------------------------------ CryptoJS (the parts people use)

  function WordArray(hex) { this._hex = hex; this.sigBytes = hex.length / 2; }
  WordArray.prototype.toString = function (enc) { return (enc || CryptoJS.enc.Hex).stringify(this); };
  function bytesOf(x) { return x instanceof WordArray ? x._hex : utf8Hex(x); }
  function digest(alg) { return function (data) { return new WordArray(global.__digest(alg, bytesOf(data), "")); }; }
  function hmac(alg) { return function (data, key) { return new WordArray(global.__digest(alg, bytesOf(data), bytesOf(key))); }; }
  var CryptoJS = {
    MD5: digest("md5"), SHA1: digest("sha1"), SHA256: digest("sha256"),
    HmacMD5: hmac("md5"), HmacSHA1: hmac("sha1"), HmacSHA256: hmac("sha256"),
    enc: {
      Hex: { stringify: function (w) { return w._hex; }, parse: function (s) { return new WordArray(toStr(s).toLowerCase()); } },
      Utf8: { stringify: function (w) { return hexUtf8(w._hex); }, parse: function (s) { return new WordArray(utf8Hex(s)); } },
      Base64: { stringify: function (w) { return hexB64(w._hex); }, parse: function (s) { return new WordArray(b64Hex(s)); } },
      Latin1: { stringify: function (w) { var o = ""; for (var i = 0; i < w._hex.length; i += 2) o += String.fromCharCode(parseInt(w._hex.substr(i, 2), 16)); return o; },
        parse: function (s) { s = toStr(s); var h = ""; for (var i = 0; i < s.length; i++) { var c = s.charCodeAt(i) & 255; h += (c < 16 ? "0" : "") + c.toString(16); } return new WordArray(h); } }
    }
  };
  global.CryptoJS = CryptoJS;

  // ------------------------------------------------------------------ lodash (the common parts)

  function iteratee(f) {
    if (typeof f === "function") return f;
    if (typeof f === "string") return function (o) { return _.get(o, f); };
    if (Array.isArray(f)) return function (o) { return _.get(o, f[0]) === f[1]; };
    if (isObj(f)) return function (o) { return Object.keys(f).every(function (k) { return deepEqual(_.get(o, k), f[k]); }); };
    return function (o) { return o; };
  }
  function pathOf(p) { return Array.isArray(p) ? p : toStr(p).replace(/\[(\w+)\]/g, ".$1").split(".").filter(function (x) { return x !== ""; }); }
  function words(s) { return toStr(s).replace(/([a-z0-9])([A-Z])/g, "$1 $2").split(/[^A-Za-z0-9]+/).filter(Boolean); }
  function list(c) { return Array.isArray(c) ? c : isObj(c) ? Object.keys(c).map(function (k) { return c[k]; }) : []; }
  var _ = {
    get: function (o, p, d) { var v = o; var ps = pathOf(p); for (var i = 0; i < ps.length; i++) { if (v === null || v === undefined) return d; v = v[ps[i]]; } return v === undefined ? d : v; },
    set: function (o, p, val) { var ps = pathOf(p), cur = o; for (var i = 0; i < ps.length - 1; i++) { if (!isObj(cur[ps[i]])) cur[ps[i]] = /^\d+$/.test(ps[i + 1]) ? [] : {}; cur = cur[ps[i]]; } cur[ps[ps.length - 1]] = val; return o; },
    has: function (o, p) { var ps = pathOf(p), v = o; for (var i = 0; i < ps.length; i++) { if (!isObj(v) || !(ps[i] in v)) return false; v = v[ps[i]]; } return true; },
    isEqual: deepEqual,
    isEmpty: function (v) { if (v === null || v === undefined) return true; if (typeof v === "string" || Array.isArray(v)) return v.length === 0; if (isObj(v)) return Object.keys(v).length === 0; return true; },
    isArray: Array.isArray, isObject: function (v) { return v !== null && (typeof v === "object" || typeof v === "function"); },
    isPlainObject: function (v) { return isObj(v) && !Array.isArray(v); },
    isString: function (v) { return typeof v === "string"; }, isNumber: function (v) { return typeof v === "number"; },
    isBoolean: function (v) { return typeof v === "boolean"; }, isFunction: function (v) { return typeof v === "function"; },
    isNil: function (v) { return v === null || v === undefined; }, isNull: function (v) { return v === null; }, isUndefined: function (v) { return v === undefined; },
    keys: function (o) { return isObj(o) ? Object.keys(o) : []; }, values: list, size: function (c) { return Array.isArray(c) || typeof c === "string" ? c.length : isObj(c) ? Object.keys(c).length : 0; },
    pick: function (o, ks) { ks = Array.isArray(ks) ? ks : Array.prototype.slice.call(arguments, 1); var r = {}; ks.forEach(function (k) { if (isObj(o) && k in o) r[k] = o[k]; }); return r; },
    omit: function (o, ks) { ks = Array.isArray(ks) ? ks : Array.prototype.slice.call(arguments, 1); var r = Object.assign({}, o); ks.forEach(function (k) { delete r[k]; }); return r; },
    map: function (c, f) { f = iteratee(f); return Array.isArray(c) ? c.map(function (v, i) { return f(v, i, c); }) : Object.keys(c || {}).map(function (k) { return f(c[k], k, c); }); },
    filter: function (c, f) { return list(c).filter(iteratee(f)); }, reject: function (c, f) { f = iteratee(f); return list(c).filter(function (v) { return !f(v); }); },
    find: function (c, f) { return list(c).find(iteratee(f)); }, findIndex: function (a, f) { return (a || []).findIndex(iteratee(f)); },
    some: function (c, f) { return list(c).some(iteratee(f)); }, every: function (c, f) { return list(c).every(iteratee(f)); },
    each: function (c, f) { if (Array.isArray(c)) c.forEach(f); else Object.keys(c || {}).forEach(function (k) { f(c[k], k, c); }); return c; },
    reduce: function (c, f, acc) { var l = list(c); return arguments.length > 2 ? l.reduce(f, acc) : l.reduce(f); },
    includes: function (c, v) { return typeof c === "string" ? c.indexOf(v) >= 0 : list(c).some(function (x) { return deepEqual(x, v); }); },
    uniq: function (a) { var out = []; (a || []).forEach(function (x) { if (!out.some(function (y) { return deepEqual(x, y); })) out.push(x); }); return out; },
    uniqBy: function (a, f) { f = iteratee(f); var seen = [], out = []; (a || []).forEach(function (x) { var k = f(x); if (!seen.some(function (y) { return deepEqual(y, k); })) { seen.push(k); out.push(x); } }); return out; },
    sortBy: function (c, fs) { fs = (Array.isArray(fs) ? fs : [fs]).map(iteratee); return list(c).slice().sort(function (a, b) { for (var i = 0; i < fs.length; i++) { var x = fs[i](a), y = fs[i](b); if (x < y) return -1; if (x > y) return 1; } return 0; }); },
    orderBy: function (c, fs, orders) { fs = (Array.isArray(fs) ? fs : [fs]).map(iteratee); orders = orders || []; return list(c).slice().sort(function (a, b) { for (var i = 0; i < fs.length; i++) { var x = fs[i](a), y = fs[i](b), d = orders[i] === "desc" ? -1 : 1; if (x < y) return -d; if (x > y) return d; } return 0; }); },
    groupBy: function (c, f) { f = iteratee(f); var r = {}; list(c).forEach(function (v) { var k = f(v); (r[k] = r[k] || []).push(v); }); return r; },
    countBy: function (c, f) { f = iteratee(f); var r = {}; list(c).forEach(function (v) { var k = f(v); r[k] = (r[k] || 0) + 1; }); return r; },
    keyBy: function (c, f) { f = iteratee(f); var r = {}; list(c).forEach(function (v) { r[f(v)] = v; }); return r; },
    mapValues: function (o, f) { f = iteratee(f); var r = {}; Object.keys(o || {}).forEach(function (k) { r[k] = f(o[k], k); }); return r; },
    mapKeys: function (o, f) { var r = {}; Object.keys(o || {}).forEach(function (k) { r[f(o[k], k)] = o[k]; }); return r; },
    chunk: function (a, n) { n = n || 1; var out = []; for (var i = 0; i < (a || []).length; i += n) out.push(a.slice(i, i + n)); return out; },
    flatten: function (a) { return [].concat.apply([], a || []); },
    flattenDeep: function (a) { return (a || []).reduce(function (acc, x) { return acc.concat(Array.isArray(x) ? _.flattenDeep(x) : x); }, []); },
    compact: function (a) { return (a || []).filter(Boolean); },
    first: function (a) { return a ? a[0] : undefined; }, last: function (a) { return a ? a[a.length - 1] : undefined; },
    take: function (a, n) { return (a || []).slice(0, n === undefined ? 1 : n); }, drop: function (a, n) { return (a || []).slice(n === undefined ? 1 : n); },
    difference: function (a, b) { return (a || []).filter(function (x) { return (b || []).indexOf(x) < 0; }); },
    intersection: function (a, b) { return (a || []).filter(function (x) { return (b || []).indexOf(x) >= 0; }); },
    union: function () { return _.uniq([].concat.apply([], arguments)); },
    without: function (a) { var ex = Array.prototype.slice.call(arguments, 1); return (a || []).filter(function (x) { return ex.indexOf(x) < 0; }); },
    zip: function () { var as = Array.prototype.slice.call(arguments), n = Math.max.apply(null, as.map(function (a) { return a.length; })); return _.range(n).map(function (i) { return as.map(function (a) { return a[i]; }); }); },
    toPairs: function (o) { return Object.keys(o || {}).map(function (k) { return [k, o[k]]; }); },
    fromPairs: function (ps) { var r = {}; (ps || []).forEach(function (p) { r[p[0]] = p[1]; }); return r; },
    sum: function (a) { return (a || []).reduce(function (s, x) { return s + x; }, 0); },
    sumBy: function (a, f) { f = iteratee(f); return (a || []).reduce(function (s, x) { return s + f(x); }, 0); },
    mean: function (a) { return a && a.length ? _.sum(a) / a.length : NaN; },
    max: function (a) { return a && a.length ? Math.max.apply(null, a) : undefined; }, min: function (a) { return a && a.length ? Math.min.apply(null, a) : undefined; },
    maxBy: function (a, f) { f = iteratee(f); return (a || []).reduce(function (m, x) { return m === undefined || f(x) > f(m) ? x : m; }, undefined); },
    minBy: function (a, f) { f = iteratee(f); return (a || []).reduce(function (m, x) { return m === undefined || f(x) < f(m) ? x : m; }, undefined); },
    range: function (a, b, step) { if (b === undefined) { b = a; a = 0; } step = step || 1; var out = []; for (var i = a; step > 0 ? i < b : i > b; i += step) out.push(i); return out; },
    times: function (n, f) { return _.range(n).map(f || function (i) { return i; }); },
    random: function (a, b) { if (b === undefined) { b = a === undefined ? 1 : a; a = 0; } return Math.floor(a + Math.random() * (b - a + 1)); },
    sample: function (a) { return a && a.length ? a[Math.floor(Math.random() * a.length)] : undefined; },
    shuffle: function (a) { var c = (a || []).slice(); for (var i = c.length - 1; i > 0; i--) { var j = Math.floor(Math.random() * (i + 1)); var t = c[i]; c[i] = c[j]; c[j] = t; } return c; },
    cloneDeep: function (v) { return v === undefined ? undefined : JSON.parse(JSON.stringify(v)); }, clone: function (v) { return Array.isArray(v) ? v.slice() : isObj(v) ? Object.assign({}, v) : v; },
    assign: Object.assign, defaults: function (o) { Array.prototype.slice.call(arguments, 1).forEach(function (s) { Object.keys(s || {}).forEach(function (k) { if (o[k] === undefined) o[k] = s[k]; }); }); return o; },
    merge: function (o) { Array.prototype.slice.call(arguments, 1).forEach(function (s) { Object.keys(s || {}).forEach(function (k) { if (isObj(o[k]) && isObj(s[k])) _.merge(o[k], s[k]); else o[k] = s[k]; }); }); return o; },
    capitalize: function (s) { s = toStr(s); return s.charAt(0).toUpperCase() + s.slice(1).toLowerCase(); },
    camelCase: function (s) { return words(s).map(function (w, i) { w = w.toLowerCase(); return i ? w.charAt(0).toUpperCase() + w.slice(1) : w; }).join(""); },
    snakeCase: function (s) { return words(s).map(function (w) { return w.toLowerCase(); }).join("_"); },
    kebabCase: function (s) { return words(s).map(function (w) { return w.toLowerCase(); }).join("-"); },
    startCase: function (s) { return words(s).map(function (w) { return w.charAt(0).toUpperCase() + w.slice(1); }).join(" "); },
    trim: function (s) { return toStr(s).trim(); }, padStart: function (s, n, c) { return toStr(s).padStart(n, c || " "); }, padEnd: function (s, n, c) { return toStr(s).padEnd(n, c || " "); },
    repeat: function (s, n) { return toStr(s).repeat(n); }, upperFirst: function (s) { s = toStr(s); return s.charAt(0).toUpperCase() + s.slice(1); },
    uniqueId: (function () { var n = 0; return function (p) { return toStr(p) + (++n); }; })(),
    noop: function () {}, identity: function (v) { return v; }
  };
  _.forEach = _.each; _.head = _.first; _.contains = _.includes; _.entries = _.toPairs; _.extend = _.assign;
  global._ = _;

  var libs = global.__libs || {};
  global.xml2Json = libs.xml2Json;
  global.moment = libs.moment;
  global.cheerio = libs.cheerio;

  function requireModule(name) {
    if (name === "lodash") return _;
    if (name === "moment") return libs.moment;
    if (name === "cheerio") return libs.cheerio;
    if (name === "crypto-js") return CryptoJS;
    if (name === "uuid") return { v4: uuid4 };
    if (name === "chai") return { expect: expect, assert: function (v, m) { if (!v) throw new AssertionError(m || "assertion failed"); } };
    throw new Error("require('" + name + "') isn't available in Gaza Man (lodash, moment, cheerio, crypto-js, uuid and chai are)");
  }

  // ------------------------------------------------------------------ pm

  function test(name, fn) {
    var entry = { name: toStr(name), passed: true, error: null, skipped: false };
    out.tests.push(entry);
    try {
      var r = typeof fn === "function" ? fn() : undefined;
      if (r && typeof r.then === "function") {
        return r.then(function () {}, function (e) { entry.passed = false; entry.error = errText(e); });
      }
    } catch (e) {
      entry.passed = false;
      entry.error = errText(e);
    }
  }
  test.skip = function (name) { out.tests.push({ name: toStr(name), passed: true, error: null, skipped: true }); };

  var pm = {
    environment: environmentScope,
    globals: globalsScope,
    collectionVariables: collectionScope,
    iterationData: dataScope,
    variables: variables,
    request: request,
    response: response,
    info: input.info,
    test: test,
    expect: expect,
    sendRequest: sendRequest,
    cookies: response ? response.cookies : new PropertyList([]),
    execution: {
      skipRequest: function () { out.skip = true; },
      setNextRequest: function (name) { out.nextRequest = name === null ? null : toStr(name); },
      location: { current: input.info.eventName }
    },
    require: requireModule
  };
  pm.setNextRequest = pm.execution.setNextRequest;

  // pm.visualizer: renders a Handlebars template with data, shown in the Visualize tab.
  pm.visualizer = {
    set: function (template, data) {
      var html = libs.handlebars ? libs.handlebars(template, data) : toStr(template);
      out.visualizer = { html: html, data: data === undefined ? null : data };
    },
    clear: function () { out.visualizer = null; }
  };

  // pm.vault: secrets kept in the workspace, never inside collections. Promise-based, like Postman.
  pm.vault = {
    get: function (k) { return Promise.resolve(own(vaultStore, toStr(k)) ? vaultStore[toStr(k)] : undefined); },
    set: function (k, v) {
      k = toStr(k); v = toStr(v);
      vaultStore[k] = v;
      out.changes.push({ op: "set", scope: "vault", key: k, value: v });
      return Promise.resolve();
    },
    unset: function (k) {
      k = toStr(k);
      delete vaultStore[k];
      out.changes.push({ op: "unset", scope: "vault", key: k });
      return Promise.resolve();
    }
  };

  // pm.cookies.jar(): the app's real cookie jar, with Node-style callbacks.
  function jarCall(op, url, args, cb) {
    var r = JSON.parse(global.__cookies(op, toStr(isObj(url) ? url.toString() : url), JSON.stringify(args || {})));
    var err = r.error ? new Error(r.error) : null;
    if (typeof cb === "function") {
      try { cb(err, r.result); } catch (e) { out.errors.push({ script: current, message: "In a cookie jar callback: " + errText(e) }); }
    }
  }
  var jar = {
    get: function (url, name, cb) { jarCall("get", url, { name: toStr(name) }, cb); },
    getAll: function (url, cb) { jarCall("getAll", url, {}, cb); },
    set: function (url, name, value, cb) {
      if (isObj(name)) { cb = value; jarCall("set", url, { name: toStr(name.name || name.key), value: toStr(name.value), path: name.path, httpOnly: !!name.httpOnly, secure: !!name.secure }, cb); }
      else jarCall("set", url, { name: toStr(name), value: toStr(value) }, cb);
    },
    unset: function (url, name, cb) { jarCall("unset", url, { name: toStr(name) }, cb); },
    clear: function (url, cb) { jarCall("clear", url, {}, cb); }
  };
  pm.cookies.jar = function () { return jar; };

  // The old `postman.*` API, still common in older collections.
  var postman = {
    setEnvironmentVariable: function (k, v) { environmentScope.set(k, v); },
    getEnvironmentVariable: function (k) { return environmentScope.get(k); },
    clearEnvironmentVariable: function (k) { environmentScope.unset(k); },
    clearEnvironmentVariables: function () { environmentScope.clear(); },
    setGlobalVariable: function (k, v) { globalsScope.set(k, v); },
    getGlobalVariable: function (k) { return globalsScope.get(k); },
    clearGlobalVariable: function (k) { globalsScope.unset(k); },
    clearGlobalVariables: function () { globalsScope.clear(); },
    setNextRequest: pm.execution.setNextRequest,
    getResponseHeader: function (k) { return response ? response.headers.get(k) : undefined; }
  };

  function makeConsole() {
    var c = {};
    ["log", "info", "warn", "error", "debug"].forEach(function (level) {
      c[level] = function () { out.logs.push({ level: level, text: logText(arguments), script: current }); };
    });
    return c;
  }
  var console = makeConsole();
  global.console = console;
  global.pm = pm;
  global.postman = postman;
  global.require = requireModule;
  // Postman runs timers after the script; we run them as soon as the script yields.
  global.setTimeout = function (fn, ms) { var args = Array.prototype.slice.call(arguments, 2); Promise.resolve().then(function () { fn.apply(null, args); }); return 0; };
  global.clearTimeout = function () {};

  var AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

  function runOne(script) {
    current = script.label;
    var tests = {};
    // The old sandbox globals. They're globals (not parameters) so that a script's own
    // `const data = …` simply shadows them, as in Postman.
    global.tests = tests;
    global.responseBody = response ? response.text() : undefined;
    global.responseCode = response ? { code: response.code, name: response.status, detail: response.status } : undefined;
    global.responseHeaders = response ? response.headers.toObject() : undefined;
    global.responseTime = response ? response.responseTime : undefined;
    global.request = { url: request.url.toString(), method: request.method, headers: request.headers.toObject(), data: request.body && request.body.raw, name: request.name };
    global.environment = environmentScope.toObject();
    global.globals = globalsScope.toObject();
    global.data = dataScope.toObject();
    global.iteration = input.info.iteration;
    function collectLegacy() {
      Object.keys(tests).forEach(function (k) {
        out.tests.push({ name: k, passed: !!tests[k], error: tests[k] ? null : "tests[\"" + k + "\"] was false", skipped: false });
      });
    }
    var fn;
    try {
      fn = new AsyncFunction(script.code);
    } catch (e) {
      out.errors.push({ script: script.label, message: errText(e) });
      return Promise.resolve();
    }
    return fn().then(collectLegacy, function (e) {
      collectLegacy();
      out.errors.push({ script: script.label, message: errText(e) });
    });
  }

  var chain = Promise.resolve();
  input.scripts.forEach(function (s) { chain = chain.then(function () { return runOne(s); }); });
  chain.then(function () { global.__done = true; });

  global.__finish = function () {
    var r = {
      method: request.method,
      url: request.url.toString(),
      headers: request.headers._items,
      body: request.body
    };
    return JSON.stringify({
      changes: out.changes, locals: locals, tests: out.tests, logs: out.logs, errors: out.errors,
      skip: out.skip, nextRequest: out.nextRequest === undefined ? "__unset__" : out.nextRequest,
      visualizer: out.visualizer === undefined ? null : out.visualizer,
      request: r, done: !!global.__done
    });
  };
})(globalThis);
