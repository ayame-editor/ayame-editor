// Ayame Editor — filename predicates shared by settings and status rendering.

export function isThemeDoc(path) {
  return !!path && /\.ayame-theme\.json$/i.test(path);
}

export function isKeymapDoc(path) {
  return !!path && /\.ayame-keys\.json$/i.test(path);
}
