//! Intl mock for QuickJS compatibility
//!
//! QuickJS does not support the Intl API natively.
//! SPA frameworks (React/Vue) commonly use Intl.DateTimeFormat/NumberFormat,
//! which would cause ReferenceError and silent failures.

use anyhow::Result;
use rquickjs::Value;

use super::JsEngine;

impl JsEngine {
    /// Setup minimal Intl mock for SPA framework compatibility.
    pub fn setup_intl(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            if (typeof globalThis.Intl !== 'undefined') return;
            globalThis.Intl = {
                DateTimeFormat: function(locale, options) {
                    return {
                        format: function(d) { return String(d); },
                        formatToParts: function(d) { return [{ type: 'literal', value: String(d) }]; },
                        resolvedOptions: function() { return { locale: locale || 'en', calendar: 'gregory', numberingSystem: 'latn' }; }
                    };
                },
                NumberFormat: function(locale, options) {
                    return {
                        format: function(n) { return String(n); },
                        formatToParts: function(n) { return [{ type: 'literal', value: String(n) }]; },
                        resolvedOptions: function() { return { locale: locale || 'en', numberingSystem: 'latn', style: 'decimal' }; }
                    };
                },
                Collator: function(locale, options) {
                    return {
                        compare: function(a, b) { return a < b ? -1 : a > b ? 1 : 0; },
                        resolvedOptions: function() { return { locale: locale || 'en', sensitivity: 'variant' }; }
                    };
                },
                PluralRules: function(locale, options) {
                    return {
                        select: function(n) { return 'other'; },
                        resolvedOptions: function() { return { locale: locale || 'en', type: 'cardinal' }; }
                    };
                },
                RelativeTimeFormat: function(locale, options) {
                    return {
                        format: function(value, unit) { return String(value) + ' ' + unit; },
                        resolvedOptions: function() { return { locale: locale || 'en', style: 'long', numeric: 'always' }; }
                    };
                },
                ListFormat: function(locale, options) {
                    return {
                        format: function(items) { return items.join(', '); },
                        resolvedOptions: function() { return { locale: locale || 'en', style: 'long', type: 'conjunction' }; }
                    };
                },
                DisplayNames: function(locale, options) {
                    return {
                        of: function(code) { return String(code); },
                        resolvedOptions: function() { return { locale: locale || 'en', type: 'language', style: 'long' }; }
                    };
                },
                Segmenter: function(locale, options) {
                    return {
                        segment: function(text) {
                            return {
                                [Symbol.iterator]: function() {
                                    var i = 0;
                                    return {
                                        next: function() {
                                            if (i < text.length) { return { value: { segment: text[i++] }, done: false }; }
                                            return { done: true };
                                        }
                                    };
                                }
                            };
                        },
                        resolvedOptions: function() { return { locale: locale || 'en', granularity: 'grapheme' }; }
                    };
                }
            };
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow::anyhow!("Failed to setup Intl mock: {:?}", e))?;
        Ok(())
    }
}
