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
