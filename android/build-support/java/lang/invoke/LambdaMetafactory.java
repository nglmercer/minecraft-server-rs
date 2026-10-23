package java.lang.invoke;

/** Compile-only signature omitted from android.jar, equivalent to the SDK's
 * core-lambda-stubs.jar. D8 desugars app lambdas; this class is never packaged. */
public final class LambdaMetafactory {
    private LambdaMetafactory() {}
    public static CallSite metafactory(MethodHandles.Lookup caller, String name,
            MethodType invokedType, MethodType samMethodType,
            MethodHandle implementation, MethodType instantiatedType) {
        throw new AssertionError("compile-only stub");
    }
}
