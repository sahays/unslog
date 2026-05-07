/* ============================================================
   NSelect — searchable native-<select> enhancement.
   Lift from glimmer (https://github.com/sahays/glimmer) with one
   addition: an `allow-custom` mode that lets the user submit a
   typed value that doesn't match any predefined option (used on
   the settings page so you can paste a brand-new model ID even
   if it isn't in the cached /models list yet).

   Markup contract:
     <select data-n-select [data-searchable] [data-allow-custom]
             [data-placeholder="..."]>
       <option value="">Placeholder text</option>
       <option value="...">...</option>
       ...
     </select>

   - `data-searchable` forces the search box on (otherwise auto-on
     when there are >5 options).
   - `data-allow-custom` enables the "Use <typed>" affordance.
   ============================================================ */

(function () {
  'use strict';

  var _SVG_CHEVRON = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>';
  var _SVG_CHECK = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';
  var _SVG_SEARCH = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>';
  var _SVG_X = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>';
  var _SVG_PLUS = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="M12 5v14"/></svg>';

  function _el(tag, attrs) {
    var e = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (k) {
        var v = attrs[k];
        if (v === undefined || v === null) return;
        if (k === 'class') e.className = v;
        else e.setAttribute(k, v);
      });
    }
    return e;
  }

  function NSelect(nativeSelect) {
    this._native = nativeSelect;
    this._name = nativeSelect.name;
    this._allowCustom = nativeSelect.hasAttribute('data-allow-custom');
    this._open = false;
    this._activeIdx = -1;
    this._readOptions();
    this._searchable = nativeSelect.hasAttribute('data-searchable') || this._options.length > 5 || this._allowCustom;
    this._build();
    this._bind();
    nativeSelect._nselect = this;
    NSelect._instances[this._name] = this;
  }

  NSelect._instances = {};
  NSelect.get = function (name) { return NSelect._instances[name] || null; };

  NSelect.initAll = function (root) {
    (root || document).querySelectorAll('select[data-n-select]').forEach(function (sel) {
      if (sel._nselect) return;
      new NSelect(sel);
    });
  };

  NSelect.prototype._readOptions = function () {
    this._options = [];
    this._placeholder = this._native.dataset.placeholder || '';
    var self = this;
    Array.from(this._native.options).forEach(function (opt) {
      if (opt.value === '') {
        if (!self._placeholder) self._placeholder = opt.textContent.trim();
      } else {
        self._options.push({ value: opt.value, label: opt.textContent.trim() });
      }
    });
  };

  NSelect.prototype._build = function () {
    var extraCls = Array.from(this._native.classList).filter(function (c) {
      return /^(w-|min-w-|max-w-|flex-|sm:|md:|lg:|xl:)/.test(c);
    }).join(' ');

    this._wrap = _el('div', { class: 'relative n-select-wrap ' + extraCls });
    this._native.parentNode.insertBefore(this._wrap, this._native);
    this._wrap.appendChild(this._native);

    this._native.style.cssText = 'position:absolute;width:1px;height:1px;opacity:0;pointer-events:none;overflow:hidden;';
    this._native.tabIndex = -1;
    this._native.setAttribute('aria-hidden', 'true');

    var curOpt = this._currentOption();

    this._triggerLabel = _el('span', { class: 'truncate' });
    this._triggerLabel.textContent = curOpt ? curOpt.label : this._placeholder;
    if (!curOpt) this._triggerLabel.style.color = 'var(--color-text-tertiary)';

    this._triggerChevron = _el('span', { class: 'n-select-chevron' });
    this._triggerChevron.innerHTML = _SVG_CHEVRON;

    this._clearBtn = _el('button', { type: 'button', class: 'n-select-clear', 'aria-label': 'Clear selection' });
    this._clearBtn.innerHTML = _SVG_X;
    this._clearBtn.style.display = curOpt ? '' : 'none';

    var rightGroup = _el('div', { class: 'n-select-trigger-right' });
    rightGroup.append(this._clearBtn, this._triggerChevron);

    this._trigger = _el('button', {
      type: 'button', role: 'combobox', 'aria-expanded': 'false', 'aria-haspopup': 'listbox',
      class: 'n-select-trigger',
    });
    if (this._native.disabled) {
      this._trigger.style.opacity = '0.5';
      this._trigger.style.pointerEvents = 'none';
    }
    this._trigger.append(this._triggerLabel, rightGroup);

    this._panel = _el('div', { role: 'listbox', class: 'n-select-panel', style: 'display:none' });

    if (this._searchable) {
      var searchWrap = _el('div', { class: 'n-select-search-wrap' });
      var searchIcon = _el('span', { class: 'n-select-search-icon' });
      searchIcon.innerHTML = _SVG_SEARCH;
      this._searchInput = _el('input', {
        type: 'text', class: 'n-select-search',
        placeholder: this._allowCustom ? 'Search or type a custom value…' : 'Search…',
        autocomplete: 'off',
      });
      searchWrap.append(searchIcon, this._searchInput);
      this._panel.appendChild(searchWrap);
    }

    this._listWrap = _el('div', { class: 'n-select-list' });
    this._panel.appendChild(this._listWrap);
    this._renderList();

    this._wrap.append(this._trigger, this._panel);
  };

  /* Find the option that matches the native <select>'s current value.
     If allow-custom is on and value isn't in the option list, fabricate
     one so the trigger label shows the custom value. */
  NSelect.prototype._currentOption = function () {
    var v = this._native.value;
    if (!v) return null;
    var found = this._options.find(function (o) { return o.value === v; });
    if (found) return found;
    if (this._allowCustom) return { value: v, label: v, _custom: true };
    return null;
  };

  NSelect.prototype._renderList = function (filter) {
    this._listWrap.innerHTML = '';
    var q = (filter || '').trim();
    var qLower = q.toLowerCase();
    var currentVal = this._native.value;

    var filtered = qLower
      ? this._options.filter(function (o) {
          return o.label.toLowerCase().indexOf(qLower) !== -1
              || o.value.toLowerCase().indexOf(qLower) !== -1;
        })
      : this._options;

    var self = this;

    /* Allow-custom: if there's a typed query that isn't an exact match,
       prepend a "Use <q>" affordance row. */
    if (this._allowCustom && q.length > 0) {
      var exact = this._options.some(function (o) { return o.value === q; });
      if (!exact) {
        var customItem = _el('div', {
          role: 'option',
          'data-value': q,
          'data-custom': 'true',
          'aria-selected': (q === currentVal) ? 'true' : 'false',
          class: 'n-select-option n-select-option-custom' + (q === currentVal ? ' selected' : ''),
        });
        var plus = _el('span', { class: 'n-select-check' });
        plus.innerHTML = _SVG_PLUS;
        var label = _el('span', { class: 'truncate' });
        label.innerHTML = 'Use &ldquo;<strong>' + escapeHTML(q) + '</strong>&rdquo;';
        customItem.append(label, plus);
        this._listWrap.appendChild(customItem);
      }
    }

    if (filtered.length === 0 && !this._allowCustom) {
      var empty = _el('div', { class: 'n-select-empty' });
      empty.textContent = 'No results found';
      this._listWrap.appendChild(empty);
      return;
    }

    filtered.forEach(function (opt) {
      var selected = opt.value === currentVal;
      var item = _el('div', {
        role: 'option', 'data-value': opt.value,
        'aria-selected': selected ? 'true' : 'false',
        class: 'n-select-option' + (selected ? ' selected' : ''),
      });
      var label = _el('span', { class: 'truncate' });
      label.textContent = opt.label;
      item.appendChild(label);
      if (selected) {
        var check = _el('span', { class: 'n-select-check' });
        check.innerHTML = _SVG_CHECK;
        item.appendChild(check);
      }
      self._listWrap.appendChild(item);
    });
  };

  function escapeHTML(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c];
    });
  }

  NSelect.prototype._syncDisplay = function () {
    var curOpt = this._currentOption();
    this._triggerLabel.textContent = curOpt ? curOpt.label : this._placeholder;
    this._triggerLabel.style.color = curOpt ? '' : 'var(--color-text-tertiary)';
    this._clearBtn.style.display = curOpt ? '' : 'none';
    this._renderList(this._searchable && this._searchInput ? this._searchInput.value : '');
  };

  /* Inject a custom value into the underlying <select> so the form
     submits it correctly, then reflect in the UI. */
  NSelect.prototype._pickCustom = function (value) {
    if (!this._native.querySelector('option[value="' + cssEscape(value) + '"]')) {
      var opt = _el('option', { value: value });
      opt.textContent = value;
      this._native.appendChild(opt);
      this._options.push({ value: value, label: value });
    }
    this._native.value = value;
    this._syncDisplay();
    this._native.dispatchEvent(new Event('change', { bubbles: true }));
  };

  function cssEscape(s) {
    if (window.CSS && CSS.escape) return CSS.escape(s);
    return String(s).replace(/["\\]/g, '\\$&');
  }

  NSelect.prototype._show = function () {
    this._open = true;
    this._trigger.setAttribute('aria-expanded', 'true');
    this._triggerChevron.classList.add('open');
    this._panel.style.display = '';

    var triggerRect = this._trigger.getBoundingClientRect();
    var panelHeight = this._panel.offsetHeight || 260;
    var spaceBelow = window.innerHeight - triggerRect.bottom;
    var spaceAbove = triggerRect.top;

    if (spaceBelow < panelHeight && spaceAbove > spaceBelow) {
      this._panel.style.bottom = this._trigger.offsetHeight + 4 + 'px';
      this._panel.style.top = 'auto';
      this._panel.classList.add('flip-up');
    } else {
      this._panel.style.top = this._trigger.offsetHeight + 4 + 'px';
      this._panel.style.bottom = 'auto';
      this._panel.classList.remove('flip-up');
    }

    var panel = this._panel;
    requestAnimationFrame(function () { panel.classList.add('open'); });
    this._activeIdx = -1;

    if (this._searchable && this._searchInput) {
      this._searchInput.value = '';
      this._renderList();
      this._searchInput.focus();
    }
  };

  NSelect.prototype._hide = function () {
    this._open = false;
    this._trigger.setAttribute('aria-expanded', 'false');
    this._triggerChevron.classList.remove('open');
    this._panel.classList.remove('open');
    var panel = this._panel;
    setTimeout(function () {
      if (!panel.classList.contains('open')) {
        panel.style.display = 'none';
        panel.style.bottom = '';
        panel.style.top = '';
        panel.classList.remove('flip-up');
      }
    }, 150);
    this._trigger.focus();
  };

  NSelect.prototype._pickItem = function (item) {
    var value = item.dataset.value;
    if (item.dataset.custom === 'true') {
      this._pickCustom(value);
    } else {
      this._native.value = value;
      this._syncDisplay();
      this._native.dispatchEvent(new Event('change', { bubbles: true }));
    }
    this._hide();
  };

  NSelect.prototype._bind = function () {
    var self = this;
    this._trigger.addEventListener('click', function (e) {
      e.stopPropagation();
      if (self._native.disabled) return;
      self._open ? self._hide() : self._show();
    });
    this._clearBtn.addEventListener('click', function (e) {
      e.stopPropagation();
      self._native.value = '';
      self._syncDisplay();
      self._native.dispatchEvent(new Event('change', { bubbles: true }));
    });
    this._listWrap.addEventListener('click', function (e) {
      var item = e.target.closest('[role="option"]');
      if (!item) return;
      self._pickItem(item);
    });
    document.addEventListener('click', function (e) {
      if (self._open && !self._wrap.contains(e.target)) self._hide();
    });
    this._wrap.addEventListener('keydown', function (e) { self._onKey(e); });
    if (this._searchable && this._searchInput) {
      this._searchInput.addEventListener('input', function () {
        self._renderList(self._searchInput.value);
        self._activeIdx = -1;
      });
      this._searchInput.addEventListener('keydown', function (e) {
        if (e.key === 'Enter') {
          e.preventDefault();
          var items = self._listWrap.querySelectorAll('[role="option"]');
          if (self._activeIdx >= 0 && items[self._activeIdx]) {
            self._pickItem(items[self._activeIdx]);
          } else if (self._allowCustom && self._searchInput.value.trim()) {
            self._pickCustom(self._searchInput.value.trim());
            self._hide();
          } else if (items.length === 1) {
            self._pickItem(items[0]);
          }
        }
      });
    }
  };

  NSelect.prototype._onKey = function (e) {
    var self = this;
    var items = function () { return self._listWrap.querySelectorAll('[role="option"]'); };
    if (!this._open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].indexOf(e.key) !== -1) { e.preventDefault(); this._show(); }
      return;
    }
    var all;
    switch (e.key) {
      case 'Escape': e.preventDefault(); this._hide(); break;
      case 'ArrowDown':
        e.preventDefault();
        this._activeIdx = Math.min(this._activeIdx + 1, items().length - 1);
        this._highlightActive(items());
        break;
      case 'ArrowUp':
        e.preventDefault();
        this._activeIdx = Math.max(this._activeIdx - 1, 0);
        this._highlightActive(items());
        break;
      case 'Enter':
        if (e.target === this._searchInput) return; /* handled in search keydown */
        e.preventDefault();
        all = items();
        if (this._activeIdx >= 0 && all[this._activeIdx]) {
          this._pickItem(all[this._activeIdx]);
        }
        break;
      case 'Tab': this._hide(); break;
      default:
        if (!this._searchable && e.key.length === 1) {
          var ch = e.key.toLowerCase(); all = items();
          for (var i = 0; i < all.length; i++) {
            if (all[i].textContent.trim().toLowerCase().charAt(0) === ch) {
              this._activeIdx = i; this._highlightActive(all); break;
            }
          }
        }
    }
  };

  NSelect.prototype._highlightActive = function (items) {
    var idx = this._activeIdx;
    Array.from(items).forEach(function (el, i) {
      el.classList.toggle('focused', i === idx);
      if (i === idx) el.scrollIntoView({ block: 'nearest' });
    });
  };

  NSelect.prototype.refresh = function () {
    this._readOptions();
    this._renderList();
    this._syncDisplay();
  };

  window.NSelect = NSelect;

  function init(root) {
    NSelect.initAll(root);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function () { init(); });
  } else {
    init();
  }
  document.addEventListener('htmx:afterSwap', function () { init(document.body); });
})();
