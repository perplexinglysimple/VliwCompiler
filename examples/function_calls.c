typedef unsigned long long u64;

__attribute__((noinline)) u64 scale_add(u64 x, u64 y);
__attribute__((noinline)) u64 mix_pair(u64 a, u64 b);

int main(void) {
    volatile u64 *in = (volatile u64 *)0x180;
    volatile u64 *out = (volatile u64 *)0x1c0;

    u64 seed = in[0];
    u64 a = scale_add(seed + 3, 5);
    u64 b = mix_pair(a, seed + 11);
    u64 c = scale_add(b, a ^ 7);

    out[0] = a;
    out[1] = b;
    out[2] = c;

    return 0;
}

__attribute__((noinline)) u64 scale_add(u64 x, u64 y) {
    return (x * 3) + (y << 2);
}

__attribute__((noinline)) u64 mix_pair(u64 a, u64 b) {
    u64 left = a ^ b;
    u64 right = (a + 9) * (b + 1);
    return left + right;
}
