const path = require('path');
const { getDefaultConfig } = require('expo/metro-config');
const root = path.resolve(__dirname, '../../../..');
const config = getDefaultConfig(__dirname);
config.watchFolders = [root];
config.resolver.assetExts = [...new Set([...config.resolver.assetExts, 'xml'])];
config.resolver.resolveRequest = (context, name, platform) => {
  const result = context.resolveRequest(context, name, platform);
  if (process.env.LORCA_CAPTURE_BEFORE === '1' && result.type === 'sourceFile' && result.filePath === path.join(root, 'mobile/app/attention.tsx')) {
    return { type: 'sourceFile', filePath: path.join(__dirname, '.native/attention-before.tsx') };
  }
  if (result.type === 'sourceFile' && result.filePath === path.join(root, 'mobile/modules/lorca-core/index.ts')) {
    return { type: 'sourceFile', filePath: path.join(__dirname, 'fixture-core.ts') };
  }
  return result;
};
module.exports = config;
