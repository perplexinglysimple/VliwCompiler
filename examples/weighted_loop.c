typedef unsigned long long u64;

int main(void) {
    volatile u64 *out = (volatile u64 *)0x100;

    u64 acc = 1;
    u64 even_sum = 0;
    u64 odd_sum = 0;

    for (u64 i = 1; i <= 12; i = i + 1) {
        u64 term = i * (i + 3);
        acc = acc + (term ^ (i << 2));
        out[5] = acc;

        if ((i & 1) == 0) {
            even_sum = even_sum + term;
            out[3] = even_sum;
        } else {
            odd_sum = odd_sum + term;
            out[4] = odd_sum;
        }
    }

    out[0] = acc;
    out[1] = even_sum;
    out[2] = odd_sum;

    return 0;
}
