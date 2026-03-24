/**
 * Conditional logger — suppressed unless KMD_DEBUG is set in localStorage.
 *
 * Enable:  localStorage.setItem('kmd:debug', '1')
 * Disable: localStorage.removeItem('kmd:debug')
 */
const isDebug = typeof localStorage !== 'undefined'
  && localStorage.getItem('kmd:debug') === '1';

export const log = {
  info: (...args: unknown[]) => { if (isDebug) console.log(...args); },
  warn: (...args: unknown[]) => { if (isDebug) console.warn(...args); },
  error: (...args: unknown[]) => { if (isDebug) console.error(...args); },
};
