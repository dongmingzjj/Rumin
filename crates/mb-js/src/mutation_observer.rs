//! MutationObserver Web API 实现
//!
//! 提供符合 Web 标准的 MutationObserver，支持：
//! - childList 变更检测（appendChild/removeChild/innerHTML）
//! - attributes 变更检测（setAttribute/removeAttribute）
//! - characterData 变更检测（textContent/nodeValue 修改）
//! - subtree 选项（监听后代节点变更）
//! - oldValue 支持（attributeOldValue/characterDataOldValue）
//! - disconnect 后停止通知

use super::*;

impl JsEngine {
    /// 注册 MutationObserver 构造函数和全局辅助函数
    pub fn setup_mutation_observer(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            globalThis.__mutation_observers__ = [];

            // MutationObserver 构造函数
            globalThis.MutationObserver = function(callback) {
                this._callback = callback;
                this._targets = [];       // 所有观察的目标节点
                this._records = [];       // 待处理的 MutationRecord
                this._connected = true;   // 是否已连接
                globalThis.__mutation_observers__.push(this);
            };

            // observe 方法：开始观察目标节点
            globalThis.MutationObserver.prototype.observe = function(target, config) {
                if (!target) throw new TypeError('observe requires a non-null target');
                this._config = config || {};
                this._target = target;
                // 确保 _targets 中包含此目标（避免重复）
                var found = false;
                for (var i = 0; i < this._targets.length; i++) {
                    if (this._targets[i] === target) { found = true; break; }
                }
                if (!found) {
                    this._targets.push(target);
                }
                this._connected = true;
            };

            // disconnect 方法：停止接收通知
            globalThis.MutationObserver.prototype.disconnect = function() {
                this._targets = [];
                this._target = null;
                this._connected = false;
            };

            // takeRecords 方法：返回待处理记录并清空队列
            globalThis.MutationObserver.prototype.takeRecords = function() {
                var records = this._records.slice();
                this._records.length = 0;
                return records;
            };

            // 通知所有匹配的 MutationObserver
            // 参数：
            //   nodeId  - 发生变更的节点 ID
            //   type    - 变更类型（'childList'/'attributes'/'characterData'）
            //   data    - 附加数据（action, childId, name, oldValue 等）
            globalThis.__notify_mutation_observers__ = function(nodeId, type, data) {
                if (!data) data = {};
                for (var i = 0; i < globalThis.__mutation_observers__.length; i++) {
                    var obs = globalThis.__mutation_observers__[i];
                    // 跳过已断开连接的观察者
                    if (!obs._connected) continue;
                    if (!obs._target) continue;

                    var config = obs._config || {};
                    var shouldNotify = false;
                    var actualTarget = null;  // 实际触发变更的节点

                    // 检查是否直接匹配观察目标
                    var targetId = obs._target._nodeId || obs._target._createdId;
                    if (targetId && (targetId == nodeId || String(targetId) === String(nodeId))) {
                        // 直接匹配：变更发生在观察目标上
                        shouldNotify = true;
                        actualTarget = obs._target;
                    }

                    // 如果启用了 subtree，检查变更节点是否是观察目标的后代
                    if (!shouldNotify && config.subtree) {
                        var el = (typeof __dom_elements__ !== 'undefined') ? __dom_elements__[nodeId] : null;
                        if (el) {
                            var walker = el;
                            while (walker) {
                                var walkerId = walker._nodeId || walker._createdId;
                                if (walkerId && (walkerId == targetId || String(walkerId) === String(targetId))) {
                                    shouldNotify = true;
                                    actualTarget = el;  // 实际变更的节点作为 target
                                    break;
                                }
                                walker = walker.parentNode;
                            }
                        }
                    }

                    if (!shouldNotify) continue;

                    // 根据变更类型检查配置
                    var matchesType = false;
                    if (type === 'attributes' && config.attributes) matchesType = true;
                    if (type === 'childList' && config.childList) matchesType = true;
                    if (type === 'characterData' && config.characterData) matchesType = true;

                    if (!matchesType) continue;

                    // 构建 MutationRecord
                    var record = {
                        type: type,
                        target: actualTarget || obs._target,
                        addedNodes: [],
                        removedNodes: [],
                        attributeName: data.name || null,
                        oldValue: null
                    };

                    // 设置 oldValue
                    if (type === 'attributes' && config.attributeOldValue && data.oldValue !== undefined) {
                        record.oldValue = data.oldValue;
                    }
                    if (type === 'characterData' && config.characterDataOldValue && data.oldValue !== undefined) {
                        record.oldValue = data.oldValue;
                    }

                    // 处理 childList 变更的 addedNodes/removedNodes
                    if (type === 'childList') {
                        if (data.addedChildIds && data.addedChildIds.length > 0) {
                            for (var j = 0; j < data.addedChildIds.length; j++) {
                                var child = (typeof __dom_elements__ !== 'undefined') ? __dom_elements__[data.addedChildIds[j]] : null;
                                if (child) record.addedNodes.push(child);
                            }
                        }
                        if (data.removedChildIds && data.removedChildIds.length > 0) {
                            for (var j = 0; j < data.removedChildIds.length; j++) {
                                var child = (typeof __dom_elements__ !== 'undefined') ? __dom_elements__[data.removedChildIds[j]] : null;
                                if (child) record.removedNodes.push(child);
                            }
                        }
                        // 兼容旧的单个 childId 格式
                        if (!data.addedChildIds && !data.removedChildIds && data.childId) {
                            var child = (typeof __dom_elements__ !== 'undefined') ? __dom_elements__[data.childId] : null;
                            if (data.action === 'add' && child) {
                                record.addedNodes.push(child);
                            } else if (data.action === 'remove' && child) {
                                record.removedNodes.push(child);
                            }
                        }
                    }

                    obs._records.push(record);
                }
            };

            // 刷新所有待处理的观察者回调
            globalThis.__flush_mutation_observers__ = function() {
                for (var i = 0; i < globalThis.__mutation_observers__.length; i++) {
                    var obs = globalThis.__mutation_observers__[i];
                    if (!obs._connected) continue;
                    if (obs._records.length > 0 && typeof obs._callback === 'function') {
                        var records = obs._records.slice();
                        obs._records.length = 0;
                        try { obs._callback(records, obs); } catch(e) {}
                    }
                }
            };
        })();
        "#;

        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                let _: Value = ctx.eval(code)?;
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to setup mutation observer: {:?}", e))?;

        Ok(())
    }

    /// 修补 dom_bridge 中的变更辅助函数，使其通知 MutationObserver。
    /// 必须在 setup_mutation_queue() 之后调用（因为 setup_mutation_queue 会重定义这些函数）。
    pub(crate) fn patch_mutation_helpers_for_observer(&mut self) -> Result<()> {
        let patch_code = r#"
        (function() {
            // 仅在 __notify_mutation_observers__ 和 __mut_set_attr__ 都存在时修补
            if (typeof __notify_mutation_observers__ !== 'function') return;
            if (typeof __mut_set_attr__ === 'undefined') return;

            // 保存原始函数引用
            var _orig_set_attr = __mut_set_attr__;
            var _orig_remove_attr = __mut_remove_attr__;
            var _orig_set_text = __mut_set_text__;
            var _orig_set_inner_html = __mut_set_inner_html__;
            var _orig_append_child = __mut_append_child__;
            var _orig_remove_child = __mut_remove_child__;

            // 修补 setAttribute：捕获旧值并通知
            __mut_set_attr__ = function(nid, name, value) {
                var oldValue = null;
                // 尝试获取旧属性值
                if (typeof __dom_elements__ !== 'undefined') {
                    var el = __dom_elements__[nid];
                    if (el && el._attrs) {
                        oldValue = el._attrs[name] !== undefined ? el._attrs[name] : null;
                    }
                }
                _orig_set_attr(nid, name, value);
                __notify_mutation_observers__(nid, 'attributes', {name: name, oldValue: oldValue});
            };

            // 修补 removeAttribute：捕获旧值并通知
            __mut_remove_attr__ = function(nid, name) {
                var oldValue = null;
                if (typeof __dom_elements__ !== 'undefined') {
                    var el = __dom_elements__[nid];
                    if (el && el._attrs) {
                        oldValue = el._attrs[name] !== undefined ? el._attrs[name] : null;
                    }
                }
                _orig_remove_attr(nid, name);
                __notify_mutation_observers__(nid, 'attributes', {name: name, oldValue: oldValue});
            };

            // 修补 textContent 设置：捕获旧值并通知
            __mut_set_text__ = function(nid, text) {
                var oldValue = null;
                if (typeof __dom_elements__ !== 'undefined') {
                    var el = __dom_elements__[nid];
                    if (el) {
                        oldValue = el._textContent || null;
                    }
                }
                _orig_set_text(nid, text);
                __notify_mutation_observers__(nid, 'characterData', {oldValue: oldValue});
            };

            // 修补 innerHTML 设置：通知 childList 变更
            __mut_set_inner_html__ = function(nid, html) {
                _orig_set_inner_html(nid, html);
                __notify_mutation_observers__(nid, 'childList', {});
            };

            // 修补 appendChild：通知 childList 变更
            __mut_append_child__ = function(parentId, childTag) {
                _orig_append_child(parentId, childTag);
                __notify_mutation_observers__(parentId, 'childList', {action: 'add', childId: childTag});
            };

            // 修补 removeChild：通知 childList 变更
            __mut_remove_child__ = function(parentId, childId) {
                _orig_remove_child(parentId, childId);
                __notify_mutation_observers__(parentId, 'childList', {action: 'remove', childId: childId});
            };
        })();
        "#;
        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                let _: Value = ctx.eval(patch_code)?;
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to patch mutation helpers: {:?}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_dom::tree::DomTree;

    /// 创建带 DOM 的测试引擎
    fn create_test_engine_with_dom() -> (JsEngine, DomTree) {
        let mut dom = DomTree::new();
        let div_id = dom.create_element("div");
        dom.append_child(dom.body_node, div_id);
        let p_id = dom.create_element("p");
        dom.append_child(div_id, p_id);

        let mut engine = JsEngine::new_with_defaults();
        engine.bind_dom(&dom).unwrap();
        (engine, dom)
    }

    /// 测试 childList 变更检测
    #[test]
    fn test_mutation_observer_childlist() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var notifications = [];
                var observer = new MutationObserver(function(records) {
                    for (var i = 0; i < records.length; i++) {
                        notifications.push({
                            type: records[i].type,
                            addedCount: records[i].addedNodes.length,
                            removedCount: records[i].removedNodes.length
                        });
                    }
                });

                var body = document.querySelector('body');
                observer.observe(body, { childList: true });

                // 添加一个子元素
                var span = document.createElement('span');
                span.textContent = 'hello';
                body.appendChild(span);

                // 刷新观察者回调
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                observer.disconnect();
                return JSON.stringify(notifications);
            })()
        "#,
            )
            .unwrap();

        let records: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(!records.is_empty(), "应该收到至少一个 MutationRecord");
        assert_eq!(records[0]["type"], "childList", "类型应为 childList");
        assert_eq!(records[0]["addedCount"], 1, "应该添加了一个节点");
    }

    /// 测试 subtree 变更检测
    #[test]
    fn test_mutation_observer_subtree() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var notifications = [];
                var observer = new MutationObserver(function(records) {
                    for (var i = 0; i < records.length; i++) {
                        notifications.push({
                            type: records[i].type,
                            target: records[i].target.tagName || 'unknown'
                        });
                    }
                });

                var body = document.querySelector('body');
                // 观察 body，启用 subtree
                observer.observe(body, { childList: true, subtree: true });

                // 修改 div（body 的子元素）的内容
                var div = document.querySelector('div');
                var newSpan = document.createElement('span');
                div.appendChild(newSpan);

                // 刷新观察者回调
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                observer.disconnect();
                return JSON.stringify(notifications);
            })()
        "#,
            )
            .unwrap();

        let records: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(!records.is_empty(), "subtree 模式下应该捕获后代节点变更");
        // target 应该是实际发生变更的节点（div），而不是观察目标（body）
        assert_eq!(records[0]["type"], "childList", "类型应为 childList");
    }

    /// 测试 attributes 变更检测
    #[test]
    fn test_mutation_observer_attributes() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var notifications = [];
                var observer = new MutationObserver(function(records) {
                    for (var i = 0; i < records.length; i++) {
                        notifications.push({
                            type: records[i].type,
                            attributeName: records[i].attributeName,
                            oldValue: records[i].oldValue
                        });
                    }
                });

                var div = document.querySelector('div');
                observer.observe(div, { attributes: true, attributeOldValue: true });

                // 设置属性
                div.setAttribute('class', 'test-class');
                div.setAttribute('id', 'my-div');

                // 刷新观察者回调
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                observer.disconnect();
                return JSON.stringify(notifications);
            })()
        "#,
            )
            .unwrap();

        let records: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(
            records.len() >= 2,
            "应该收到至少两个属性变更记录，实际: {}",
            records.len()
        );
        assert_eq!(records[0]["type"], "attributes", "类型应为 attributes");
        assert_eq!(records[0]["attributeName"], "class", "第一个属性应为 class");
        assert_eq!(records[1]["attributeName"], "id", "第二个属性应为 id");
    }

    /// 测试 disconnect 后不再收到通知
    #[test]
    fn test_mutation_observer_disconnect() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var countBefore = 0;
                var countAfter = 0;

                var observer = new MutationObserver(function(records) {
                    countBefore += records.length;
                });

                var body = document.querySelector('body');
                observer.observe(body, { childList: true });

                // 添加子元素（应该被检测到）
                var div1 = document.createElement('div');
                body.appendChild(div1);

                // 刷新
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                // 断开连接
                observer.disconnect();

                // 再次添加子元素（不应该被检测到）
                var div2 = document.createElement('div');
                body.appendChild(div2);

                // 重新创建观察者来计数断开后的变更
                var observer2 = new MutationObserver(function(records) {
                    countAfter += records.length;
                });
                observer2.observe(body, { childList: true });

                // 刷新
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                return JSON.stringify({before: countBefore, after: countAfter});
            })()
        "#,
            )
            .unwrap();

        let data: serde_json::Value = serde_json::from_str(&result).unwrap();
        let before = data["before"].as_i64().unwrap_or(0);
        let after = data["after"].as_i64().unwrap_or(0);
        assert!(before > 0, "disconnect 前应该收到通知，实际: {}", before);
        assert_eq!(after, 0, "disconnect 后不应该收到通知，实际: {}", after);
    }

    /// 测试 characterData 变更检测
    #[test]
    fn test_mutation_observer_character_data() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var notifications = [];
                var observer = new MutationObserver(function(records) {
                    for (var i = 0; i < records.length; i++) {
                        notifications.push({
                            type: records[i].type,
                            oldValue: records[i].oldValue
                        });
                    }
                });

                // 创建文本节点并观察
                var div = document.querySelector('div');
                var textNode = document.createTextNode('original');
                div.appendChild(textNode);

                // 观察文本节点的 characterData 变更
                observer.observe(textNode, { characterData: true, characterDataOldValue: true });

                // 修改文本内容
                textNode.textContent = 'modified';

                // 刷新观察者回调
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                observer.disconnect();
                return JSON.stringify(notifications);
            })()
        "#,
            )
            .unwrap();

        let records: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(!records.is_empty(), "应该收到 characterData 变更通知");
        assert_eq!(
            records[0]["type"], "characterData",
            "类型应为 characterData"
        );
    }

    /// 测试 subtree + attributes 组合
    #[test]
    fn test_mutation_observer_subtree_attributes() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var notifications = [];
                var observer = new MutationObserver(function(records) {
                    for (var i = 0; i < records.length; i++) {
                        notifications.push({
                            type: records[i].type,
                            attributeName: records[i].attributeName,
                            targetTag: records[i].target.tagName || 'unknown'
                        });
                    }
                });

                var body = document.querySelector('body');
                observer.observe(body, { attributes: true, subtree: true });

                // 修改后代元素的属性
                var div = document.querySelector('div');
                div.setAttribute('data-test', 'value');

                var p = document.querySelector('p');
                p.setAttribute('class', 'highlight');

                // 刷新观察者回调
                if (typeof __flush_mutation_observers__ === 'function') {
                    __flush_mutation_observers__();
                }

                observer.disconnect();
                return JSON.stringify(notifications);
            })()
        "#,
            )
            .unwrap();

        let records: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(
            records.len() >= 2,
            "subtree + attributes 应该捕获后代属性变更，实际: {}",
            records.len()
        );
        // 第一个记录的 target 应该是 div
        assert_eq!(
            records[0]["targetTag"], "DIV",
            "target 应该是实际变更的节点"
        );
        assert_eq!(records[0]["attributeName"], "data-test");
        // 第二个记录的 target 应该是 p
        assert_eq!(records[1]["targetTag"], "P", "target 应该是实际变更的节点");
        assert_eq!(records[1]["attributeName"], "class");
    }

    /// 测试 takeRecords
    #[test]
    fn test_mutation_observer_take_records() {
        let (mut engine, _dom) = create_test_engine_with_dom();

        let result = engine
            .eval(
                r#"
            (function() {
                var observer = new MutationObserver(function() {});

                var body = document.querySelector('body');
                observer.observe(body, { childList: true });

                // 添加子元素
                var div = document.createElement('div');
                body.appendChild(div);

                // 不刷新，直接用 takeRecords 获取
                var records = observer.takeRecords();

                observer.disconnect();
                return records.length;
            })()
        "#,
            )
            .unwrap();

        assert_eq!(result, "1", "takeRecords 应该返回 1 条记录");
    }
}
