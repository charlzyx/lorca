// Fixture-only native startup hook for a headless Simulator. Calls the development
// runtime's public embedded-bundle loader; no application/core data is read.
#import <Foundation/Foundation.h>
#import <dispatch/dispatch.h>
#import <objc/message.h>
static void loadFixture(int attempt) {
  Class cls = NSClassFromString(@"EXDevLauncherController");
  if (cls && [cls respondsToSelector:NSSelectorFromString(@"sharedInstance")]) {
    id controller = ((id (*)(id, SEL))objc_msgSend)(cls, NSSelectorFromString(@"sharedInstance"));
    SEL selector = NSSelectorFromString(@"loadLocalBundleOnSuccess:onError:");
    if (controller && [controller respondsToSelector:selector]) {
      ((void (*)(id, SEL, id, id))objc_msgSend)(controller, selector,
        ^{ NSLog(@"Attention fixture embedded bundle loaded"); },
        ^(NSError *error){ NSLog(@"Attention fixture load error: %@", error); });
      return;
    }
  }
  if (attempt < 10) dispatch_after(dispatch_time(DISPATCH_TIME_NOW, NSEC_PER_SEC), dispatch_get_main_queue(), ^{ loadFixture(attempt + 1); });
}
__attribute__((constructor)) static void startFixture(void) {
  [[NSUserDefaults standardUserDefaults] setBool:YES forKey:@"EXDevMenuIsOnboardingFinished"];
  [[NSUserDefaults standardUserDefaults] setBool:NO forKey:@"EXDevMenuShowsAtLaunch"];
  [[NSUserDefaults standardUserDefaults] setBool:NO forKey:@"EXDevMenuShowFloatingActionButton"];
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{ loadFixture(0); });
}
