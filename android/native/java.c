// Minimal headless JNI launcher. Code is installed by Android, never executed
// from writable app storage. Runtime configuration comes from the app process.
#include <jni.h>
#include <dlfcn.h>
#include <limits.h>
#include <malloc.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void fail(const char *message) { fprintf(stderr, "mcpanel-java: %s\n", message); exit(1); }
int main(int argc, char **argv) {
    // HotSpot uses pointer bits internally; disable Bionic heap pointer tags
    // before the VM allocates anything (supported by Android 11+).
    if (!mallopt(M_BIONIC_SET_HEAP_TAGGING_LEVEL, 0)) fail("cannot disable heap pointer tagging");
    const char *home = getenv("MCPANEL_ANDROID_JAVA_HOME");
    if (!home || home[0] != '/') fail("missing absolute runtime home");
    char native[PATH_MAX], library[PATH_MAX], java_home[PATH_MAX + 32];
    ssize_t n = readlink("/proc/self/exe", native, sizeof(native) - 1);
    if (n <= 0) fail("cannot locate installed launcher");
    native[n] = 0;
    char *slash = strrchr(native, '/');
    if (!slash) fail("invalid launcher path");
    *slash = 0;
    snprintf(library, sizeof(library), "%s/lib/server/libjvm.so", home);
    void *handle = dlopen(library, RTLD_NOW | RTLD_GLOBAL);
    if (!handle) fail(dlerror());
    jint (*create)(JavaVM **, void **, void *) = dlsym(handle, "JNI_CreateJavaVM");
    if (!create) fail("JNI_CreateJavaVM unavailable");
    int jar = 0;
    for (int i = 1; i < argc; i++) if (!strcmp(argv[i], "-jar")) { jar = i; break; }
    int version = argc == 2 && (!strcmp(argv[1], "-version") || !strcmp(argv[1], "--version"));
    if (!version && (!jar || jar + 1 >= argc)) fail("expected JVM options followed by -jar JAR [arguments]");
    JavaVMOption *options = calloc((size_t)argc + 8, sizeof(*options));
    if (!options) fail("out of memory");
    int count = 0;
    for (int i = 1; i < jar; i++) options[count++].optionString = argv[i];
    snprintf(java_home, sizeof(java_home), "-Djava.home=%s", home);
    char boot[PATH_MAX + 64], libs[PATH_MAX + 64], classpath[PATH_MAX + 64];
    char user_home[PATH_MAX + 64], temp[PATH_MAX + 64];
    snprintf(boot, sizeof(boot), "-Dsun.boot.library.path=%s/lib", home);
    snprintf(libs, sizeof(libs), "-Djava.library.path=%s/lib", home);
    options[count++].optionString = java_home;
    options[count++].optionString = boot;
    options[count++].optionString = libs;
    options[count++].optionString = "-Djava.awt.headless=true";
    options[count++].optionString = "-Xshare:off";
    if (getenv("HOME")) {
        snprintf(user_home, sizeof(user_home), "-Duser.home=%s", getenv("HOME"));
        options[count++].optionString = user_home;
    }
    if (getenv("TMPDIR")) {
        snprintf(temp, sizeof(temp), "-Djava.io.tmpdir=%s", getenv("TMPDIR"));
        options[count++].optionString = temp;
    }
    if (!version) {
        if (strlen(argv[jar + 1]) >= PATH_MAX) fail("jar path too long");
        snprintf(classpath, sizeof(classpath), "-Djava.class.path=%s", argv[jar + 1]);
        options[count++].optionString = classpath;
    }
    JavaVMInitArgs init = { .version = JNI_VERSION_1_8, .nOptions = count, .options = options, .ignoreUnrecognized = JNI_FALSE };
    JavaVM *vm;
    JNIEnv *env;
    if (create(&vm, (void **)&env, &init) != JNI_OK) fail("JVM startup failed");
    int result = 0;
    if (version) {
        jclass system = (*env)->FindClass(env, "java/lang/System");
        jmethodID property = (*env)->GetStaticMethodID(env, system, "getProperty", "(Ljava/lang/String;)Ljava/lang/String;");
        jstring key = (*env)->NewStringUTF(env, "java.runtime.version");
        jstring value = (*env)->CallStaticObjectMethod(env, system, property, key);
        const char *text = (*env)->GetStringUTFChars(env, value, NULL);
        fprintf(stderr, "openjdk version \"%s\" (MCPANEL Android)\n", text);
        (*env)->ReleaseStringUTFChars(env, value, text);
    } else {
        jclass helper = (*env)->FindClass(env, "sun/launcher/LauncherHelper");
        if (!helper) goto exception;
        jmethodID check = (*env)->GetStaticMethodID(env, helper, "checkAndLoadMain", "(ZILjava/lang/String;)Ljava/lang/Class;");
        if (!check) goto exception;
        jstring path = (*env)->NewStringUTF(env, argv[jar + 1]);
        jclass main_class = (*env)->CallStaticObjectMethod(env, helper, check, JNI_FALSE, 2, path);
        if ((*env)->ExceptionCheck(env) || !main_class) goto exception;
        jmethodID main = (*env)->GetStaticMethodID(env, main_class, "main", "([Ljava/lang/String;)V");
        if (!main) goto exception;
        jclass string_class = (*env)->FindClass(env, "java/lang/String");
        jobjectArray args = (*env)->NewObjectArray(env, argc - jar - 2, string_class, NULL);
        if (!args) goto exception;
        for (int i = jar + 2; i < argc; i++) {
            jstring arg = (*env)->NewStringUTF(env, argv[i]);
            (*env)->SetObjectArrayElement(env, args, i - jar - 2, arg);
            (*env)->DeleteLocalRef(env, arg);
        }
        (*env)->CallStaticVoidMethod(env, main_class, main, args);
exception:
        if ((*env)->ExceptionCheck(env)) { (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); result = 1; }
    }
    (*vm)->DestroyJavaVM(vm);
    free(options);
    return result;
}
