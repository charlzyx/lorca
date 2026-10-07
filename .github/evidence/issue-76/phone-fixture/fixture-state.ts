// Synthetic view projections only. The real phone screen/store/forms are imported unchanged.
import { useStore } from '../../../../mobile/src/core/store';
const fixture = require('../fixture.json');
export function setScenario(scenario = 'active') {
  const attention = structuredClone(fixture.attention);
  if (scenario === 'followups') {
    attention.items = attention.items.filter((item: any) => item.category === 'commitment' || item.category === 'change');
    attention.briefs = [];
    attention.preferences.summaries = false;
  } else if (scenario === 'empty') {
    attention.items = []; attention.briefs = []; attention.preferences.summaries = false;
  }
  useStore.setState({ attention, bots: fixture.bots, chats: fixture.chats, ready: true });
}
