// Finds visible elements for a selector and tags the one at `index` with a
// unique attribute so CDP can address it natively. Returns the match count.
(function (kind, query, index, token) {
  const visible = (el) => {
    const style = window.getComputedStyle(el);
    return style.visibility !== 'hidden' && style.display !== 'none' && el.getClientRects().length > 0;
  };
  const byText = (needle) => {
    const lower = needle.toLowerCase();
    const all = Array.from(document.querySelectorAll('body *'));
    const hits = all.filter((el) => (el.innerText || '').toLowerCase().includes(lower));
    // Keep only the innermost matches.
    return hits.filter((el) => !hits.some((other) => other !== el && el.contains(other)));
  };
  let matches;
  if (kind === 'css') {
    matches = Array.from(document.querySelectorAll(query));
  } else if (kind === 'xpath') {
    const result = document.evaluate(query, document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
    matches = Array.from({ length: result.snapshotLength }, (_, i) => result.snapshotItem(i));
  } else {
    matches = byText(query);
  }
  matches = matches.filter((el) => el instanceof Element && visible(el));
  if (token && index < matches.length) {
    document.querySelectorAll('[data-autologin]').forEach((el) => el.removeAttribute('data-autologin'));
    matches[index].setAttribute('data-autologin', token);
  }
  return matches.length;
})
