// Sets an input's value through the native setter so React/Angular/Vue see
// the change, then fires the events those frameworks listen for.
(function (token, value) {
  const el = document.querySelector('[data-autologin="' + token + '"]');
  if (!el) return false;
  el.focus();
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
  setter.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
})
