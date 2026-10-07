// Clicks a tagged element the way a finger would: scroll it into view, then
// fire the pointer, mouse and click events that login forms listen for.
// Used where there is no native input channel (mobile WebView).
(function (token) {
  const el = document.querySelector('[data-autologin="' + token + '"]');
  if (!el) return false;
  el.scrollIntoView({ block: 'center', inline: 'center' });
  const rect = el.getBoundingClientRect();
  const at = { bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 };
  el.dispatchEvent(new PointerEvent('pointerdown', { ...at, pointerType: 'touch', isPrimary: true }));
  el.dispatchEvent(new MouseEvent('mousedown', at));
  if (typeof el.focus === 'function') el.focus();
  el.dispatchEvent(new PointerEvent('pointerup', { ...at, pointerType: 'touch', isPrimary: true }));
  el.dispatchEvent(new MouseEvent('mouseup', at));
  el.click();
  return true;
})
