import { Stack, ThemeProvider, DefaultTheme } from 'expo-router';
import { useEffect } from 'react';
import { hideMenu } from 'expo-dev-menu';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
export default function FixtureLayout() {
  useEffect(() => { const timer = setTimeout(() => hideMenu(), 1200); return () => clearTimeout(timer); }, []);
  return <GestureHandlerRootView style={{ flex: 1 }}><ThemeProvider value={DefaultTheme}>
    <Stack screenOptions={{ headerTintColor: '#007AFF', contentStyle: { backgroundColor: '#F2F2F7' } }}>
      <Stack.Screen name="index" options={{ headerShown: false }} />
      <Stack.Screen name="attention" options={{ title: 'Attention', presentation: 'formSheet', sheetAllowedDetents: [1], headerShown: true, headerShadowVisible: false, headerStyle: { backgroundColor: '#F2F2F7' }, contentStyle: { backgroundColor: '#F2F2F7' } }} />
    </Stack>
  </ThemeProvider></GestureHandlerRootView>;
}
