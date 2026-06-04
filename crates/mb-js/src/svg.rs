//! SVG DOM API 支持
//!
//! 提供 SVGElement 及常用 SVG 子类型（SVGSVGElement、SVGCircleElement 等）。
//! createElementNS 由 dom_bridge.rs 中的 document 对象直接提供。

use super::*;

impl JsEngine {
    /// 注册 SVGElement 及相关 SVG DOM API
    /// 在 run_setup() 中调用，位于 setup_misc() 之后（确保 HTMLElement 已注册）
    pub fn setup_svg(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            'use strict';

            // ---- SVGElement：继承自 HTMLElement ----
            function SVGElement() {}
            SVGElement.prototype = Object.create(
                (typeof HTMLElement !== 'undefined' ? HTMLElement : function(){}).prototype
            );
            SVGElement.prototype.constructor = SVGElement;

            // SVG 元素通用属性 className（在 SVG 中是 SVGAnimatedString）
            Object.defineProperty(SVGElement.prototype, 'className', {
                get: function() {
                    var self = this;
                    return {
                        baseVal: (self._attrs && self._attrs['class']) || '',
                        animVal: (self._attrs && self._attrs['class']) || ''
                    };
                },
                set: function(v) {
                    if (this._attrs) this._attrs['class'] = String(v);
                },
                configurable: true,
                enumerable: true
            });

            // ownerSVGElement — 指向最近的 <svg> 祖先（简化为 null）
            Object.defineProperty(SVGElement.prototype, 'ownerSVGElement', {
                get: function() { return null; },
                configurable: true
            });

            // viewportElement
            Object.defineProperty(SVGElement.prototype, 'viewportElement', {
                get: function() { return null; },
                configurable: true
            });

            // SVG 常用方法 — 返回空/默认值
            SVGElement.prototype.getCTM = function() {
                return {
                    a: 1, b: 0, c: 0, d: 1, e: 0, f: 0,
                    m11: 1, m12: 0, m13: 0, m14: 0,
                    m21: 0, m22: 1, m23: 0, m24: 0,
                    m31: 0, m32: 0, m33: 1, m34: 0,
                    m41: 0, m42: 0, m43: 0, m44: 1,
                    inverse: function() { return this; },
                    multiply: function() { return this; },
                    translate: function() { return this; },
                    scale: function() { return this; },
                    rotate: function() { return this; },
                    flipX: function() { return this; },
                    flipY: function() { return this; },
                    toString: function() { return 'matrix(1,0,0,1,0,0)'; }
                };
            };

            SVGElement.prototype.getScreenCTM = function() {
                return this.getCTM();
            };

            SVGElement.prototype.getBBox = function() {
                return { x: 0, y: 0, width: 0, height: 0 };
            };

            // getComputedTextLength（SVGTextContentElement）
            SVGElement.prototype.getComputedTextLength = function() { return 0; };

            // getTotalLength（SVGPathElement 等）
            SVGElement.prototype.getTotalLength = function() { return 0; };

            // getPointAtLength
            SVGElement.prototype.getPointAtLength = function() {
                return { x: 0, y: 0 };
            };

            // ---- 常用 SVG 元素子类型 ----

            // SVGSVGElement — <svg> 根元素
            function SVGSVGElement() {}
            SVGSVGElement.prototype = Object.create(SVGElement.prototype);
            SVGSVGElement.prototype.constructor = SVGSVGElement;

            SVGSVGElement.prototype.createSVGRect = function() {
                return { x: 0, y: 0, width: 0, height: 0 };
            };
            SVGSVGElement.prototype.createSVGPoint = function() {
                return { x: 0, y: 0 };
            };
            SVGSVGElement.prototype.createSVGMatrix = function() {
                return {
                    a: 1, b: 0, c: 0, d: 1, e: 0, f: 0,
                    m11: 1, m12: 0, m13: 0, m14: 0,
                    m21: 0, m22: 1, m23: 0, m24: 0,
                    m31: 0, m32: 0, m33: 1, m34: 0,
                    m41: 0, m42: 0, m43: 0, m44: 1
                };
            };

            // SVGCircleElement — <circle>
            function SVGCircleElement() {}
            SVGCircleElement.prototype = Object.create(SVGElement.prototype);
            SVGCircleElement.prototype.constructor = SVGCircleElement;

            // SVGRectElement — <rect>
            function SVGRectElement() {}
            SVGRectElement.prototype = Object.create(SVGElement.prototype);
            SVGRectElement.prototype.constructor = SVGRectElement;

            // SVGPathElement — <path>
            function SVGPathElement() {}
            SVGPathElement.prototype = Object.create(SVGElement.prototype);
            SVGPathElement.prototype.constructor = SVGPathElement;

            // SVGEllipseElement — <ellipse>
            function SVGEllipseElement() {}
            SVGEllipseElement.prototype = Object.create(SVGElement.prototype);
            SVGEllipseElement.prototype.constructor = SVGEllipseElement;

            // SVGLineElement — <line>
            function SVGLineElement() {}
            SVGLineElement.prototype = Object.create(SVGElement.prototype);
            SVGLineElement.prototype.constructor = SVGLineElement;

            // SVGPolylineElement — <polyline>
            function SVGPolylineElement() {}
            SVGPolylineElement.prototype = Object.create(SVGElement.prototype);
            SVGPolylineElement.prototype.constructor = SVGPolylineElement;

            // SVGPolygonElement — <polygon>
            function SVGPolygonElement() {}
            SVGPolygonElement.prototype = Object.create(SVGElement.prototype);
            SVGPolygonElement.prototype.constructor = SVGPolygonElement;

            // SVGTextElement — <text>
            function SVGTextElement() {}
            SVGTextElement.prototype = Object.create(SVGElement.prototype);
            SVGTextElement.prototype.constructor = SVGTextElement;

            // SVGGElement — <g>
            function SVGGElement() {}
            SVGGElement.prototype = Object.create(SVGElement.prototype);
            SVGGElement.prototype.constructor = SVGGElement;

            // SVGDefsElement — <defs>
            function SVGDefsElement() {}
            SVGDefsElement.prototype = Object.create(SVGElement.prototype);
            SVGDefsElement.prototype.constructor = SVGDefsElement;

            // SVGUseElement — <use>
            function SVGUseElement() {}
            SVGUseElement.prototype = Object.create(SVGElement.prototype);
            SVGUseElement.prototype.constructor = SVGUseElement;

            // SVGClipPathElement — <clipPath>
            function SVGClipPathElement() {}
            SVGClipPathElement.prototype = Object.create(SVGElement.prototype);
            SVGClipPathElement.prototype.constructor = SVGClipPathElement;

            // SVGLinearGradientElement — <linearGradient>
            function SVGLinearGradientElement() {}
            SVGLinearGradientElement.prototype = Object.create(SVGElement.prototype);
            SVGLinearGradientElement.prototype.constructor = SVGLinearGradientElement;

            // SVGStopElement — <stop>
            function SVGStopElement() {}
            SVGStopElement.prototype = Object.create(SVGElement.prototype);
            SVGStopElement.prototype.constructor = SVGStopElement;

            // SVGPatternElement — <pattern>
            function SVGPatternElement() {}
            SVGPatternElement.prototype = Object.create(SVGElement.prototype);
            SVGPatternElement.prototype.constructor = SVGPatternElement;

            // SVGMaskElement — <mask>
            function SVGMaskElement() {}
            SVGMaskElement.prototype = Object.create(SVGElement.prototype);
            SVGMaskElement.prototype.constructor = SVGMaskElement;

            // SVGSymbolElement — <symbol>
            function SVGSymbolElement() {}
            SVGSymbolElement.prototype = Object.create(SVGElement.prototype);
            SVGSymbolElement.prototype.constructor = SVGSymbolElement;

            // SVGImageElement — <image>
            function SVGImageElement() {}
            SVGImageElement.prototype = Object.create(SVGElement.prototype);
            SVGImageElement.prototype.constructor = SVGImageElement;

            // SVGForeignObjectElement — <foreignObject>
            function SVGForeignObjectElement() {}
            SVGForeignObjectElement.prototype = Object.create(SVGElement.prototype);
            SVGForeignObjectElement.prototype.constructor = SVGForeignObjectElement;

            // ---- 注册到全局 ----
            globalThis.SVGElement = SVGElement;
            globalThis.SVGSVGElement = SVGSVGElement;
            globalThis.SVGCircleElement = SVGCircleElement;
            globalThis.SVGRectElement = SVGRectElement;
            globalThis.SVGPathElement = SVGPathElement;
            globalThis.SVGEllipseElement = SVGEllipseElement;
            globalThis.SVGLineElement = SVGLineElement;
            globalThis.SVGPolylineElement = SVGPolylineElement;
            globalThis.SVGPolygonElement = SVGPolygonElement;
            globalThis.SVGTextElement = SVGTextElement;
            globalThis.SVGGElement = SVGGElement;
            globalThis.SVGDefsElement = SVGDefsElement;
            globalThis.SVGUseElement = SVGUseElement;
            globalThis.SVGClipPathElement = SVGClipPathElement;
            globalThis.SVGLinearGradientElement = SVGLinearGradientElement;
            globalThis.SVGStopElement = SVGStopElement;
            globalThis.SVGPatternElement = SVGPatternElement;
            globalThis.SVGMaskElement = SVGMaskElement;
            globalThis.SVGSymbolElement = SVGSymbolElement;
            globalThis.SVGImageElement = SVGImageElement;
            globalThis.SVGForeignObjectElement = SVGForeignObjectElement;

            // SVG 元素标签名到构造函数的映射（供 dom_bridge 的 createElementNS 使用）
            globalThis.__svg_element_map__ = {
                'svg': SVGSVGElement,
                'circle': SVGCircleElement,
                'rect': SVGRectElement,
                'path': SVGPathElement,
                'ellipse': SVGEllipseElement,
                'line': SVGLineElement,
                'polyline': SVGPolylineElement,
                'polygon': SVGPolygonElement,
                'text': SVGTextElement,
                'g': SVGGElement,
                'defs': SVGDefsElement,
                'use': SVGUseElement,
                'clippath': SVGClipPathElement,
                'lineargradient': SVGLinearGradientElement,
                'radialgradient': SVGLinearGradientElement,
                'stop': SVGStopElement,
                'pattern': SVGPatternElement,
                'mask': SVGMaskElement,
                'symbol': SVGSymbolElement,
                'image': SVGImageElement,
                'foreignobject': SVGForeignObjectElement,
                'title': SVGElement,
                'desc': SVGElement,
                'metadata': SVGElement,
                'a': SVGGElement,
                'switch': SVGElement,
                'marker': SVGElement
            };

            // 为 SVG 原型链添加 style 属性支持
            Object.defineProperty(SVGElement.prototype, 'style', {
                get: function() {
                    if (!this._style) this._style = {};
                    return this._style;
                },
                configurable: true
            });

            // 为 SVG 原型链添加 dataset 属性支持
            Object.defineProperty(SVGElement.prototype, 'dataset', {
                get: function() {
                    var ds = {};
                    var attrs = this._attrs || {};
                    for (var k in attrs) {
                        if (k.startsWith('data-')) {
                            var key = k.substring(5).replace(/-([a-z])/g, function(m, c) { return c.toUpperCase(); });
                            ds[key] = attrs[k];
                        }
                    }
                    return ds;
                },
                configurable: true
            });

            // ---- SVGElement.prototype 的 DOM 属性方法混入 ----
            // 确保 SVG 元素也有 getAttribute / setAttribute 等常见 DOM 方法
            // 这些方法在 bind_dom 之后通过 __dom_element_proto__ 可用，
            // 但 SVG 子类型需要从 SVGElement.prototype 继承
            // 注意：实际混入在 bind_dom 后通过 createElementNS 中的代码完成

        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup SVG: {:?}", e))?;
        Ok(())
    }
}
