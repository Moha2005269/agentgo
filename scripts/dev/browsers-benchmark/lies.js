// CreepJS's lie tests (src/lies/index.ts, queryLies) for the functions
// AgentGo's fingerprint protection wraps. Returns {name: [failed tests]}.
(() => {
  const RAND = Math.random();
  const failsTypeError = ({ spawnErr, withStack, final }) => {
    try { spawnErr(); throw Error(); } catch (err) {
      if (err.constructor.name != 'TypeError') return true;
      return withStack ? withStack(err) : false;
    } finally { final && final(); }
  };
  const failsWithError = (fn) => { try { fn(); return false; } catch (e) { return true; } };
  const known = (name) => ({ [`function ${name}() { [native code] }`]: true, [`function get ${name}() { [native code] }`]: true, [`function () { [native code] }`]: true });
  const stackAt = (err, reg, i = 1) => reg.test(String(err.stack).split('\n')[i]);
  const query = (apiFunction, proto, obj) => {
    const name = apiFunction.name.replace(/get\s/, '');
    const nativeProto = Object.getPrototypeOf(apiFunction);
    const lies = {
      'illegal error': !!obj && failsTypeError({ spawnErr: () => obj.prototype[name] }),
      'call interface': failsTypeError({ spawnErr: () => { new apiFunction(); apiFunction.call(proto); } }),
      'apply interface': failsTypeError({ spawnErr: () => { new apiFunction(); apiFunction.apply(proto); } }),
      'new instance': failsTypeError({ spawnErr: () => new apiFunction() }),
      'class extends': failsTypeError({ spawnErr: () => { class Fake extends apiFunction {} } }),
      'null conversion': failsTypeError({ spawnErr: () => Object.setPrototypeOf(apiFunction, null).toString(), final: () => Object.setPrototypeOf(apiFunction, nativeProto) }),
      'toString': !known(name)[Function.prototype.toString.call(apiFunction)] || !known('toString')[Function.prototype.toString.call(apiFunction.toString)],
      'prototype in': 'prototype' in apiFunction,
      'descriptor': !!(Object.getOwnPropertyDescriptor(apiFunction, 'arguments') || Object.getOwnPropertyDescriptor(apiFunction, 'caller') || Object.getOwnPropertyDescriptor(apiFunction, 'prototype') || Object.getOwnPropertyDescriptor(apiFunction, 'toString')),
      'own keys': Reflect.ownKeys(apiFunction).sort().toString() != 'length,name',
      'object toString error': failsTypeError({ spawnErr: () => Object.create(apiFunction).toString(), withStack: (e) => !stackAt(e, /at Function\.toString /) }) || failsTypeError({ spawnErr: () => Object.create(new Proxy(apiFunction, {})).toString(), withStack: (e) => !stackAt(e, /at Object\.toString/) }),
      'incompatible proxy': failsTypeError({ spawnErr: () => { apiFunction.arguments; apiFunction.caller; } }),
      'toString incompatible proxy': failsTypeError({ spawnErr: () => { apiFunction.toString.arguments; apiFunction.toString.caller; } }),
      'too much recursion': failsTypeError({ spawnErr: () => { Object.setPrototypeOf(apiFunction, Object.create(apiFunction)).toString(); }, final: () => Object.setPrototypeOf(apiFunction, nativeProto) }),
    };
    const p1 = new Proxy(apiFunction, {}), p2 = new Proxy(apiFunction, {}), p3 = new Proxy(apiFunction, {});
    Object.assign(lies, {
      'recursion __proto__': !failsTypeError({ spawnErr: () => { apiFunction.__proto__ = p1; apiFunction++; }, final: () => Object.setPrototypeOf(apiFunction, nativeProto) }),
      'chain cycle': !failsTypeError({ spawnErr: () => { Object.setPrototypeOf(p1, Object.create(p1)).toString(); }, final: () => Object.setPrototypeOf(p1, nativeProto) }),
      'chain cycle __proto__': !failsTypeError({ spawnErr: () => { p2.__proto__ = p2; p2++; }, final: () => Object.setPrototypeOf(p2, nativeProto) }),
      'reflect set proto': failsTypeError({ spawnErr: () => { Reflect.setPrototypeOf(apiFunction, Object.create(apiFunction)); RAND in apiFunction; throw new TypeError(); }, final: () => Object.setPrototypeOf(apiFunction, nativeProto) }),
      'reflect set proto proxy': !failsTypeError({ spawnErr: () => { Reflect.setPrototypeOf(p3, Object.create(p3)); RAND in p3; }, final: () => Object.setPrototypeOf(p3, nativeProto) }),
      'instanceof': failsTypeError({ spawnErr: () => { apiFunction instanceof apiFunction; }, withStack: (e) => !stackAt(e, /at (Function\.)?\[Symbol.hasInstance\]/) }) || failsTypeError({ spawnErr: () => { const p = new Proxy(apiFunction, {}); p instanceof p; }, withStack: (e) => !stackAt(e, /at (Proxy\.)?\[Symbol.hasInstance\]/) }),
      'define properties': failsWithError(() => { Object.defineProperty(apiFunction, '', { configurable: true }).toString(); Reflect.deleteProperty(apiFunction, ''); }),
    });
    return Object.keys(lies).filter((k) => lies[k]);
  };
  const getter = (o, k) => Object.getOwnPropertyDescriptor(o.prototype, k).get;
  const targets = {
    'Function.toString': [Function.prototype.toString, Function.prototype, Function],
    'HTMLCanvasElement.toDataURL': [HTMLCanvasElement.prototype.toDataURL, HTMLCanvasElement.prototype, HTMLCanvasElement],
    'HTMLCanvasElement.toBlob': [HTMLCanvasElement.prototype.toBlob, HTMLCanvasElement.prototype, HTMLCanvasElement],
    'CanvasRenderingContext2D.getImageData': [CanvasRenderingContext2D.prototype.getImageData, CanvasRenderingContext2D.prototype, CanvasRenderingContext2D],
    'WebGLRenderingContext.readPixels': [WebGLRenderingContext.prototype.readPixels, WebGLRenderingContext.prototype, WebGLRenderingContext],
    'AudioBuffer.getChannelData': [AudioBuffer.prototype.getChannelData, AudioBuffer.prototype, AudioBuffer],
    'HTMLIFrameElement.contentWindow': [getter(HTMLIFrameElement, 'contentWindow'), HTMLIFrameElement.prototype, HTMLIFrameElement],
    'Element.innerHTML(set)': [Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML').set, Element.prototype, Element],
    'Node.appendChild': [Node.prototype.appendChild, Node.prototype, Node],
    'Document.write': [Document.prototype.write, Document.prototype, Document],
    'window.open': [window.open, Window.prototype, Window],
  };
  const out = {};
  for (const [name, [fn, proto, obj]] of Object.entries(targets)) { try { out[name] = query(fn, proto, obj); } catch (e) { out[name] = ['threw ' + e]; } }
  return JSON.stringify(out);
})()
