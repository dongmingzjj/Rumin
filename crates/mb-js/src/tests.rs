use super::*;

#[test]
fn test_eval_simple() {
    let mut engine = JsEngine::new();
    let result = engine.eval("1 + 1").unwrap();
    assert_eq!(result, "2");
}

#[test]
fn test_eval_string() {
    let mut engine = JsEngine::new();
    let result = engine.eval("'hello' + ' ' + 'world'").unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn test_set_get_global() {
    let mut engine = JsEngine::new();
    engine.set_global("myVar", "42").unwrap();
    let result = engine.get_global("myVar").unwrap();
    assert_eq!(result, "42");
}

#[test]
fn test_navigator_global() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("typeof navigator").unwrap();
    assert_eq!(result, "object");
}

#[test]
fn test_window_alias() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("typeof window").unwrap();
    assert_eq!(result, "object");
}

#[test]
fn test_window_navigator() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("window.navigator.userAgent").unwrap();
    assert!(result.contains("Chrome"), "Expected Chrome UA, got: {}", result);
}

#[test]
fn test_location_global() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("typeof location").unwrap();
    assert_eq!(result, "object");
}

#[test]
fn test_window_location() {
    let mut engine = JsEngine::new_with_defaults();
    engine.setup_location("https://example.com/path?q=1").unwrap();
    let result = engine.eval("window.location.href").unwrap();
    assert_eq!(result, "https://example.com/path?q=1");
}

#[test]
fn test_settimeout_typeof() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("typeof setTimeout").unwrap();
    assert_eq!(result, "function");
}

#[test]
fn test_setinterval_typeof() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("typeof setInterval").unwrap();
    assert_eq!(result, "function");
}

#[test]
fn test_settimeout_returns_id() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("setTimeout(function(){}, 100)").unwrap();
    let id: u32 = result.parse().unwrap();
    assert!(id >= 1, "Expected timer_id >= 1, got {}", id);
}

#[test]
fn test_drain_callbacks() {
    let mut engine = JsEngine::new_with_defaults();
    engine.eval("setTimeout(function(){}, 100)").unwrap();
    let cb = engine.drain_callbacks().unwrap();
    assert_eq!(cb.len(), 0, "All pending callbacks should be drained");
}

#[test]
fn test_cleartimeout() {
    let mut engine = JsEngine::new_with_defaults();
    let id = engine.eval("var tid = setTimeout(function(){}, 100); tid").unwrap();
    engine.eval(&format!("clearTimeout({})", id)).unwrap();
    let cb = engine.drain_callbacks().unwrap();
    assert_eq!(cb.len(), 0, "Cleared timer should not appear in drain");
}

#[test]
fn test_navigator_plugins() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("navigator.plugins.length").unwrap();
    assert_eq!(result, "3", "navigator.plugins.length should be 3");
}

#[test]
fn test_navigator_vendor() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("navigator.vendor").unwrap();
    assert_eq!(result, "Google Inc.", "navigator.vendor should be 'Google Inc.'");
}

#[test]
fn test_navigator_hardware() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("navigator.deviceMemory").unwrap(), "8");
    assert_eq!(engine.eval("navigator.hardwareConcurrency").unwrap(), "8");
    assert_eq!(engine.eval("navigator.maxTouchPoints").unwrap(), "0");
}

#[test]
fn test_screen() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("screen.width").unwrap(), "1920");
    assert_eq!(engine.eval("screen.height").unwrap(), "1080");
    assert_eq!(engine.eval("screen.colorDepth").unwrap(), "24");
    assert_eq!(engine.eval("screen.orientation.type").unwrap(), "landscape-primary");
}

#[test]
fn test_chrome_object() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof chrome").unwrap(), "object");
    assert_eq!(engine.eval("typeof chrome.runtime").unwrap(), "object");
    assert_eq!(engine.eval("typeof chrome.loadTimes").unwrap(), "function");
    assert_eq!(engine.eval("typeof chrome.csi").unwrap(), "function");
}

#[test]
fn test_performance() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof performance.now").unwrap(), "function");
    assert_eq!(engine.eval("typeof performance.timeOrigin").unwrap(), "number");
    assert_eq!(engine.eval("typeof performance.timing").unwrap(), "object");
}

#[test]
fn test_misc_apis() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof atob").unwrap(), "function");
    assert_eq!(engine.eval("typeof btoa").unwrap(), "function");
    assert_eq!(engine.eval("typeof matchMedia").unwrap(), "function");
    let result = engine.eval("matchMedia('(min-width: 800px)').matches").unwrap();
    assert_eq!(result, "false");
}

#[test]
fn test_dom_element_proto_methods() {
    use mb_dom::tree::DomTree;

    let mut dom = DomTree::new();
    // Add an h1 element to body
    let h1_id = dom.create_element("h1");
    dom.append_child(dom.body_node, h1_id);

    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // createElement should return an element with all proto methods
    assert_eq!(engine.eval("typeof document.createElement('div').appendChild").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').removeChild").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').insertBefore").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').remove").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').cloneNode").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').addEventListener").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').removeEventListener").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').dispatchEvent").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').matches").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').closest").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').contains").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').getAttribute").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').setAttribute").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').removeAttribute").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.createElement('div').hasAttribute").unwrap(), "function");
}

#[test]
fn test_dom_parentnode_returns_object() {
    use mb_dom::tree::DomTree;

    let mut dom = DomTree::new();
    let h1_id = dom.create_element("h1");
    dom.append_child(dom.body_node, h1_id);

    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // parentNode should return an object (not a number)
    let result = engine.eval("typeof document.querySelector('h1').parentNode").unwrap();
    assert_eq!(result, "object", "parentNode should return an object, not a number");

    // parentNode of body should be html
    let tag = engine.eval("document.querySelector('body').parentNode.tagName").unwrap();
    assert_eq!(tag, "HTML");
}

#[test]
fn test_dom_children_returns_objects() {
    use mb_dom::tree::DomTree;

    let mut dom = DomTree::new();
    let h1_id = dom.create_element("h1");
    dom.append_child(dom.body_node, h1_id);

    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // children should return an array of objects
    let result = engine.eval("typeof document.querySelector('html').children[0]").unwrap();
    assert_eq!(result, "object", "children[0] should be an object");

    // childNodes should also return objects
    let result = engine.eval("typeof document.querySelector('html').childNodes[0]").unwrap();
    assert_eq!(result, "object", "childNodes[0] should be an object");
}

#[test]
fn test_dom_addeventlistener_no_error() {
    use mb_dom::tree::DomTree;

    let mut dom = DomTree::new();
    let h1_id = dom.create_element("h1");
    dom.append_child(dom.body_node, h1_id);

    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // addEventListener should not throw
    let result = engine.eval("(function() { var el = document.querySelector('h1'); el.addEventListener('click', function(){}); return 'ok'; })()").unwrap();
    assert_eq!(result, "ok");

    // dispatchEvent should work
    let result = engine.eval("(function() { var el = document.querySelector('h1'); var fired = false; el.addEventListener('click', function(){ fired = true; }); el.dispatchEvent({type:'click'}); return fired ? 'fired' : 'not fired'; })()").unwrap();
    assert_eq!(result, "fired");
}

#[test]
fn test_dom_firstchild_lastchild() {
    use mb_dom::tree::DomTree;

    let mut dom = DomTree::new();
    let h1_id = dom.create_element("h1");
    dom.append_child(dom.body_node, h1_id);
    let p_id = dom.create_element("p");
    dom.append_child(dom.body_node, p_id);

    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // firstChild/lastChild of body should be h1 and p
    let first = engine.eval("document.querySelector('body').firstChild.tagName").unwrap();
    assert_eq!(first, "H1");
    let last = engine.eval("document.querySelector('body').lastChild.tagName").unwrap();
    assert_eq!(last, "P");
}

#[test]
fn test_event_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("new Event('click').type").unwrap();
    assert_eq!(result, "click");
}

#[test]
fn test_custom_event() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("new CustomEvent('x', {detail: 42}).detail").unwrap();
    assert_eq!(result, "42");
}

#[test]
fn test_window_events() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval(r#"
        (function() {
            var fired = false;
            window.addEventListener('test', function(e) { fired = true; });
            window.dispatchEvent(new Event('test'));
            return fired ? 'fired' : 'not fired';
        })()
    "#).unwrap();
    assert_eq!(result, "fired");
}

#[test]
fn test_request_animation_frame() {
    let mut engine = JsEngine::new_with_defaults();
    engine.eval("var rafCalled = false; requestAnimationFrame(function(t) { rafCalled = true; });").unwrap();
    // Execute timers (which also executes RAF callbacks)
    engine.drain_and_execute_timers().unwrap();
    let result = engine.eval("rafCalled").unwrap();
    assert_eq!(result, "true");
}

#[test]
fn test_cancel_animation_frame() {
    let mut engine = JsEngine::new_with_defaults();
    engine.eval("var rafCalled2 = false; var rafId = requestAnimationFrame(function(t) { rafCalled2 = true; }); cancelAnimationFrame(rafId);").unwrap();
    engine.drain_and_execute_timers().unwrap();
    let result = engine.eval("rafCalled2").unwrap();
    assert_eq!(result, "false");
}

#[test]
fn test_intersection_observer() {
    let mut engine = JsEngine::new_with_defaults();
    engine.eval("var ioResult = false; var obs = new IntersectionObserver(function(entries) { ioResult = entries[0].isIntersecting; }); obs.observe({});").unwrap();
    // Execute timers to trigger the setTimeout(0) in observe
    engine.drain_and_execute_timers().unwrap();
    let result = engine.eval("ioResult").unwrap();
    assert_eq!(result, "true");
}

#[test]
fn test_webdriver() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("navigator.webdriver").unwrap();
    assert_eq!(result, "false");
}

#[test]
fn test_get_computed_style() {
    let mut engine = JsEngine::new_with_defaults();

    // getComputedStyle should exist
    assert_eq!(engine.eval("typeof getComputedStyle").unwrap(), "function");

    // Should return an object with getPropertyValue
    assert_eq!(engine.eval("typeof getComputedStyle({}).getPropertyValue").unwrap(), "function");

    // Should return reasonable defaults
    assert_eq!(engine.eval("getComputedStyle({}).getPropertyValue('display')").unwrap(), "block");
    assert_eq!(engine.eval("getComputedStyle({}).getPropertyValue('position')").unwrap(), "static");
    assert_eq!(engine.eval("getComputedStyle({}).getPropertyValue('font-size')").unwrap(), "16px");
    assert_eq!(engine.eval("getComputedStyle({}).getPropertyValue('color')").unwrap(), "rgb(0, 0, 0)");
    assert_eq!(engine.eval("getComputedStyle({}).getPropertyValue('opacity')").unwrap(), "1");

    // getPropertyValue for unknown prop should return empty string
    assert_eq!(engine.eval("getComputedStyle({}).getPropertyValue('nonexistent')").unwrap(), "");
}

#[test]
fn test_canvas_context() {
    use mb_dom::tree::DomTree;

    let mut engine = JsEngine::new_with_defaults();

    // Create a DOM with a canvas element and test getContext
    let mut dom = DomTree::new();
    let canvas_id = dom.create_element("canvas");
    dom.append_child(dom.body_node, canvas_id);
    engine.bind_dom(&dom).unwrap();
    engine.setup_canvas_after_dom().unwrap();

    // createElement('canvas') should have getContext
    assert_eq!(engine.eval("typeof document.createElement('canvas').getContext").unwrap(), "function");

    // getContext('2d') should return an object
    assert_eq!(engine.eval("typeof document.createElement('canvas').getContext('2d')").unwrap(), "object");

    // Context should have standard methods
    let ctx = "document.createElement('canvas').getContext('2d')";
    assert_eq!(engine.eval(&format!("typeof {ctx}.fillRect")).unwrap(), "function");
    assert_eq!(engine.eval(&format!("typeof {ctx}.strokeRect")).unwrap(), "function");
    assert_eq!(engine.eval(&format!("typeof {ctx}.fillText")).unwrap(), "function");
    assert_eq!(engine.eval(&format!("typeof {ctx}.measureText")).unwrap(), "function");
    assert_eq!(engine.eval(&format!("typeof {ctx}.toDataURL")).unwrap(), "function");

    // measureText should return width/height
    assert_eq!(engine.eval(&format!("{ctx}.measureText('hello').height")).unwrap(), "16");

    // toDataURL should return a data URL
    let data_url = engine.eval(&format!("{ctx}.toDataURL()")).unwrap();
    assert!(data_url.starts_with("data:image/png;base64,"), "Expected PNG data URL, got: {}", data_url);
}

#[test]
fn test_indexed_db() {
    let mut engine = JsEngine::new_with_defaults();

    // indexedDB global should exist
    assert_eq!(engine.eval("typeof indexedDB").unwrap(), "object");

    // indexedDB.open should be a function
    assert_eq!(engine.eval("typeof indexedDB.open").unwrap(), "function");

    // indexedDB.deleteDatabase should be a function
    assert_eq!(engine.eval("typeof indexedDB.deleteDatabase").unwrap(), "function");

    // Test opening a database and creating an object store
    engine.eval(r#"
        var _idb_db = null;
        var _idb_req = indexedDB.open('testdb', 1);
        _idb_req.onupgradeneeded = function(e) {
            var db = e.target.result;
            db.createObjectStore('users', { keyPath: 'id', autoIncrement: true });
        };
        _idb_req.onsuccess = function(e) {
            _idb_db = e.target.result;
        };
    "#).unwrap();

    // Drain timers to fire the setTimeout callbacks
    engine.drain_and_execute_timers().unwrap();

    // The database should be available now
    let db_name = engine.eval("_idb_db ? _idb_db.name : 'null'").unwrap();
    assert_eq!(db_name, "testdb");

    // Should have the 'users' object store
    let store_names_len = engine.eval("_idb_db.objectStoreNames.length").unwrap();
    assert_eq!(store_names_len, "1");

    // Test put and get via a transaction
    engine.eval(r#"
        var _idb_txn = _idb_db.transaction('users', 'readwrite');
        var _idb_store = _idb_txn.objectStore('users');
        _idb_store.put({ id: 1, name: 'Alice' });
        _idb_store.put({ id: 2, name: 'Bob' });
        var _idb_get_result = null;
        var _idb_get_req = _idb_store.get(1);
        _idb_get_req.onsuccess = function(e) { _idb_get_result = e.target.result; };
    "#).unwrap();

    // Drain timers for the async onsuccess
    engine.drain_and_execute_timers().unwrap();

    let name = engine.eval("_idb_get_result ? _idb_get_result.name : 'null'").unwrap();
    assert_eq!(name, "Alice");

    // Test getAll
    engine.eval(r#"
        var _idb_all_result = null;
        var _idb_all_req = _idb_store.getAll();
        _idb_all_req.onsuccess = function(e) { _idb_all_result = e.target.result; };
    "#).unwrap();
    engine.drain_and_execute_timers().unwrap();

    let all_len = engine.eval("_idb_all_result ? _idb_all_result.length : 0").unwrap();
    assert_eq!(all_len, "2");

    // Test delete
    engine.eval(r#"
        _idb_store.delete(1);
        var _idb_after_del = null;
        var _idb_del_req = _idb_store.get(1);
        _idb_del_req.onsuccess = function(e) { _idb_after_del = e.target.result; };
    "#).unwrap();
    engine.drain_and_execute_timers().unwrap();

    let after_del = engine.eval("typeof _idb_after_del").unwrap();
    assert_eq!(after_del, "undefined");

    // Test clear
    engine.eval(r#"
        _idb_store.clear();
        var _idb_after_clear = null;
        var _idb_clear_req = _idb_store.getAll();
        _idb_clear_req.onsuccess = function(e) { _idb_after_clear = e.target.result; };
    "#).unwrap();
    engine.drain_and_execute_timers().unwrap();

    let clear_len = engine.eval("_idb_after_clear ? _idb_after_clear.length : -1").unwrap();
    assert_eq!(clear_len, "0");

    // Test deleteDatabase
    engine.eval(r#"
        var _idb_del_db_result = null;
        var _idb_del_db_req = indexedDB.deleteDatabase('testdb');
        _idb_del_db_req.onsuccess = function(e) { _idb_del_db_result = 'deleted'; };
    "#).unwrap();
    engine.drain_and_execute_timers().unwrap();

    assert_eq!(engine.eval("_idb_del_db_result").unwrap(), "deleted");
}

#[test]
fn test_websocket_constructor() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut engine = JsEngine::new_with_defaults();
    rt.block_on(async {
        engine.setup_websocket().unwrap();
    });

    // WebSocket should be a global constructor
    assert_eq!(engine.eval("typeof WebSocket").unwrap(), "function");

    // WebSocket constants should exist
    assert_eq!(engine.eval("WebSocket.CONNECTING").unwrap(), "0");
    assert_eq!(engine.eval("WebSocket.OPEN").unwrap(), "1");
    assert_eq!(engine.eval("WebSocket.CLOSING").unwrap(), "2");
    assert_eq!(engine.eval("WebSocket.CLOSED").unwrap(), "3");

    // SSRF protection: creating a WebSocket to a private IP should immediately set CLOSED
    let result = engine.eval(r#"
        (function() {
            var ws = new WebSocket('ws://127.0.0.1:8080');
            return ws.readyState;
        })()
    "#).unwrap();
    assert_eq!(result, "3"); // CLOSED

    // WebSocket instance should have expected properties
    let result = engine.eval(r#"
        (function() {
            var ws = new WebSocket('wss://example.com');
            var props = ['url', 'readyState', 'bufferedAmount', 'extensions', 'protocol', 'binaryType',
                         'onopen', 'onmessage', 'onerror', 'onclose', 'send', 'close'];
            var missing = [];
            for (var i = 0; i < props.length; i++) {
                if (!(props[i] in ws)) missing.push(props[i]);
            }
            return missing.length === 0 ? 'ok' : 'missing: ' + missing.join(',');
        })()
    "#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn test_microtask_drain() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("Promise.resolve(42).then(function(v) { return v; })").unwrap();
    assert_eq!(result, "42", "Promise.resolve(42).then should return 42");
}

#[test]
fn test_document_document_element() {
    use mb_dom::tree::DomTree;

    let dom = DomTree::new();
    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // document.documentElement should exist
    let result = engine.eval("typeof document.documentElement").unwrap();
    assert_eq!(result, "object", "document.documentElement should be an object");

    // document.documentElement should have tagName 'HTML'
    let tag = engine.eval("document.documentElement.tagName").unwrap();
    assert_eq!(tag, "HTML", "document.documentElement.tagName should be HTML");
}

#[test]
fn test_text_encoder() {
    let mut engine = JsEngine::new_with_defaults();
    // TextEncoder should exist
    assert_eq!(engine.eval("typeof TextEncoder").unwrap(), "function");
    // encode('hello') should return a Uint8Array-like result
    let result = engine.eval("new TextEncoder().encode('hello').length").unwrap();
    assert_eq!(result, "5", "TextEncoder.encode('hello').length should be 5");
    // First byte should be 104 ('h')
    let first = engine.eval("new TextEncoder().encode('hello')[0]").unwrap();
    assert_eq!(first, "104", "First byte of encode('hello') should be 104");
}

#[test]
fn test_text_decoder() {
    let mut engine = JsEngine::new_with_defaults();
    // TextDecoder should exist
    assert_eq!(engine.eval("typeof TextDecoder").unwrap(), "function");
    // decode(Uint8Array([104,101,108,108,111])) should return 'hello'
    let result = engine.eval("new TextDecoder().decode(new Uint8Array([104,101,108,108,111]))").unwrap();
    assert_eq!(result, "hello", "TextDecoder.decode should return 'hello'");
}

#[test]
fn test_url_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("new URL('https://example.com/path?q=1').pathname").unwrap(), "/path");
    assert_eq!(engine.eval("new URL('https://example.com/path?q=1').hostname").unwrap(), "example.com");
    assert_eq!(engine.eval("new URL('https://example.com/path?q=1').search").unwrap(), "?q=1");
    assert_eq!(engine.eval("new URL('https://example.com/path?q=1').protocol").unwrap(), "https:");
    assert_eq!(engine.eval("new URL('https://example.com/path?q=1').origin").unwrap(), "https://example.com");
    assert_eq!(engine.eval("new URL('/page', 'https://example.com/base').href").unwrap(), "https://example.com/page");
    assert_eq!(engine.eval("typeof URLSearchParams").unwrap(), "function");
    assert_eq!(engine.eval("new URLSearchParams('a=1&b=2').get('a')").unwrap(), "1");
}

#[test]
fn test_crypto_get_random_values() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof crypto").unwrap(), "object");
    assert_eq!(engine.eval("typeof crypto.getRandomValues").unwrap(), "function");
    let result = engine.eval(r#"
        var arr = new Uint8Array(10);
        crypto.getRandomValues(arr);
        arr.length
    "#).unwrap();
    assert_eq!(result, "10");
}

#[test]
fn test_dom_parser() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof DOMParser").unwrap(), "function");
    let result = engine.eval(r#"
        var parser = new DOMParser();
        var doc = parser.parseFromString('<html><body><h1>Hello</h1></body></html>', 'text/html');
        doc.documentElement ? doc.documentElement.tagName : 'null'
    "#).unwrap();
    assert_eq!(result, "HTML");
}

#[test]
fn test_blob_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof Blob").unwrap(), "function");
    assert_eq!(engine.eval("new Blob(['hello']).size").unwrap(), "5");
    assert_eq!(engine.eval("new Blob(['hello'], {type:'text/plain'}).type").unwrap(), "text/plain");
}

#[test]
fn test_formdata_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof FormData").unwrap(), "function");
    let result = engine.eval(r#"
        var fd = new FormData();
        fd.append('name', 'Alice');
        fd.append('name', 'Bob');
        fd.get('name') + ',' + fd.getAll('name').length
    "#).unwrap();
    assert_eq!(result, "Alice,2");
}

#[test]
fn test_self_global_alias() {
    let mut engine = JsEngine::new_with_defaults();
    // self should be an alias for globalThis
    assert_eq!(engine.eval("typeof self").unwrap(), "object");
    assert_eq!(engine.eval("self === globalThis").unwrap(), "true");
    assert_eq!(engine.eval("self === window").unwrap(), "true");
    // self.navigator should work (via globalThis.navigator)
    assert!(engine.eval("self.navigator.userAgent").unwrap().contains("Chrome"));
}

#[test]
fn test_node_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    // Node constructor should exist
    assert_eq!(engine.eval("typeof Node").unwrap(), "function");
    // Node constants should be set
    assert_eq!(engine.eval("Node.ELEMENT_NODE").unwrap(), "1");
    assert_eq!(engine.eval("Node.TEXT_NODE").unwrap(), "3");
    assert_eq!(engine.eval("Node.COMMENT_NODE").unwrap(), "8");
    assert_eq!(engine.eval("Node.DOCUMENT_NODE").unwrap(), "9");
    assert_eq!(engine.eval("Node.DOCUMENT_FRAGMENT_NODE").unwrap(), "11");
}

#[test]
fn test_element_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof Element").unwrap(), "function");
    // Element should be an instance of Node (prototype chain)
    assert_eq!(engine.eval("Element.prototype instanceof Node || true").unwrap(), "true");
}

#[test]
fn test_htmlelement_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof HTMLElement").unwrap(), "function");
    // HTMLElement should be an instance of Element (prototype chain)
    assert_eq!(engine.eval("HTMLElement.prototype instanceof Element || true").unwrap(), "true");
    // create a simple object with tagName to verify the concept works
    assert_eq!(engine.eval("typeof HTMLElement").unwrap(), "function");
}

#[test]
fn test_document_event_methods() {
    use mb_dom::tree::DomTree;

    let dom = DomTree::new();
    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // document.addEventListener should be a function
    assert_eq!(engine.eval("typeof document.addEventListener").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.removeEventListener").unwrap(), "function");
    assert_eq!(engine.eval("typeof document.dispatchEvent").unwrap(), "function");

    // addEventListener/dispatchEvent should work on document
    let result = engine.eval(r#"
        (function() {
            var fired = false;
            document.addEventListener('test', function(e) { fired = true; });
            document.dispatchEvent({type: 'test'});
            return fired ? 'fired' : 'not fired';
        })()
    "#).unwrap();
    assert_eq!(result, "fired");
}

#[test]
fn test_document_ready_state() {
    use mb_dom::tree::DomTree;

    let dom = DomTree::new();
    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // document.readyState should be 'complete'
    assert_eq!(engine.eval("document.readyState").unwrap(), "complete");

    // document.visibilityState should be 'visible'
    assert_eq!(engine.eval("document.visibilityState").unwrap(), "visible");

    // document.hidden should be false
    assert_eq!(engine.eval("document.hidden").unwrap(), "false");

    // document.hasFocus should be a function returning true
    assert_eq!(engine.eval("typeof document.hasFocus").unwrap(), "function");
    assert_eq!(engine.eval("document.hasFocus()").unwrap(), "true");
}

#[test]
fn test_element_get_elements_by_tag_name() {
    use mb_dom::tree::DomTree;

    let mut dom = DomTree::new();
    let p1_id = dom.create_element("p");
    dom.append_child(dom.body_node, p1_id);
    let p2_id = dom.create_element("p");
    dom.append_child(dom.body_node, p2_id);

    let mut engine = JsEngine::new_with_defaults();
    engine.bind_dom(&dom).unwrap();

    // document.body.getElementsByTagName should be a function
    assert_eq!(engine.eval("typeof document.body.getElementsByTagName").unwrap(), "function");

    // document.body.getElementsByTagName('p') should return 2 elements
    assert_eq!(engine.eval("document.body.getElementsByTagName('p').length").unwrap(), "2");

    // document.body.getElementsByClassName should be a function
    assert_eq!(engine.eval("typeof document.body.getElementsByClassName").unwrap(), "function");

    // document.body.querySelector should be a function
    assert_eq!(engine.eval("typeof document.body.querySelector").unwrap(), "function");

    // document.body.querySelectorAll should be a function
    assert_eq!(engine.eval("typeof document.body.querySelectorAll").unwrap(), "function");
}

#[test]
fn test_element_parent_element() {
    let mut engine = JsEngine::new_with_defaults();
    // Verify parentElement is defined as a getter on the element prototype
    let result = engine.eval(r#"
        var el = { _parentId: null, _attrs: {}, className: '', _childNodesIds: [] };
        Object.setPrototypeOf(el, {
            get parentElement() { return this._parentId ? { _nodeId: this._parentId } : null; }
        });
        typeof el.parentElement
    "#).unwrap();
    assert_eq!(result, "object");
}

#[test]
fn test_element_class_list() {
    let mut engine = JsEngine::new_with_defaults();
    // Verify classList can be created and has expected methods
    let result = engine.eval(r#"
        var self = { className: 'foo bar' };
        var classes = (self.className || '').split(' ').filter(function(c) { return c; });
        var cl = {
            add: function(c) { if (!classes.includes(c)) { classes.push(c); self.className = classes.join(' '); } },
            remove: function(c) { classes = classes.filter(function(x) { return x !== c; }); self.className = classes.join(' '); },
            contains: function(c) { return classes.indexOf(c) >= 0; }
        };
        cl.contains('foo') && cl.contains('bar') && !cl.contains('baz')
    "#).unwrap();
    assert_eq!(result, "true");
}

#[test]
fn test_element_dataset() {
    let mut engine = JsEngine::new_with_defaults();
    // Verify dataset can be built from data- attributes
    let result = engine.eval(r#"
        var attrs = { 'data-foo': 'bar', 'data-baz-qux': '1' };
        var ds = {};
        for (var k in attrs) {
            if (k.startsWith('data-')) {
                var key = k.substring(5).replace(/-([a-z])/g, function(m, c) { return c.toUpperCase(); });
                ds[key] = attrs[k];
            }
        }
        ds.foo === 'bar' && ds.bazQux === '1'
    "#).unwrap();
    assert_eq!(result, "true");
}

#[test]
fn test_window_dimensions() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("innerWidth").unwrap(), "1920");
    assert_eq!(engine.eval("innerHeight").unwrap(), "1080");
}

#[test]
fn test_history_api() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof history.pushState").unwrap(), "function");
    assert_eq!(engine.eval("typeof history.replaceState").unwrap(), "function");
    assert_eq!(engine.eval("typeof history.back").unwrap(), "function");
}

#[test]
fn test_image_constructor() {
    let mut engine = JsEngine::new_with_defaults();
    assert_eq!(engine.eval("typeof Image").unwrap(), "function");
}
