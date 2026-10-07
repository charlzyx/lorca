// In-memory stand-in at the core API boundary. Never starts Rust, pairs, or contacts a relay.
import { useStore } from '../../../../mobile/src/core/store';
export async function request<T = unknown>(method: string, params: any = {}): Promise<T> {
  const attention = structuredClone(useStore.getState().attention);
  if (method === 'attention.preferences') Object.assign(attention.preferences, params);
  else if (method === 'attention.resolve') attention.items = attention.items.filter(item => item.id !== params.id);
  else throw new Error(`Fixture does not implement ${method}`);
  useStore.setState({ attention });
  return attention as T;
}
