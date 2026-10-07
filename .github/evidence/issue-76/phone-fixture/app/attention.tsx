import { useLocalSearchParams } from 'expo-router';
import { useEffect } from 'react';
import AttentionScreen from '../../../../../mobile/app/attention';
import { setScenario } from '../fixture-state';
import { initialScenario } from '../scenario';
setScenario(initialScenario);
export default function FixtureAttention() {
  const { scenario } = useLocalSearchParams<{ scenario?: string }>();
  useEffect(() => setScenario(scenario ?? initialScenario), [scenario]);
  return <AttentionScreen />;
}
