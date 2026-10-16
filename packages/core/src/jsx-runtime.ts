export const Fragment = Symbol.for('keyfill.fragment');

export function jsx(type: any, props: Record<string, any>): any {
  if (type === Fragment) return props.children;
  if (typeof type !== 'function') {
    throw new Error(`keyfill: строчных элементов нет, импортируй примитив из @keyfill/core (получено <${String(type)}>)`);
  }
  return type(props);
}

export const jsxs = jsx;
