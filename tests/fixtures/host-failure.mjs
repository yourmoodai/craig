// Copied into a temporary plugin only; never enabled in the shipped runtime.
if (process.argv[2] !== 'rewrite') throw Error('invalid_test_command');
for await (const chunk of process.stdin) { void chunk; }
if (process.env.SAVIOR_HOST_SCENARIO === 'worker_exit') process.exitCode = 73;
else if (process.env.SAVIOR_HOST_SCENARIO === 'processing_timeout') {
  process.stdout.write(JSON.stringify({version:1,status:'error',code:'processing_timeout'})+'\n');
  process.exitCode = 1;
} else throw Error('invalid_test_scenario');
