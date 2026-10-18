import { useRef, useState } from 'react';
import { getErrorMessage } from '../utils/errors';
import { INTELLIGENCE_TESTS, gradeIntelligenceResponse } from '../utils/intelligenceTests';

// The 降智 test runs in this session, one per API profile or subscription account; a new run
// replaces the previous one. The ticked questions are sent at once and each shows up as soon
// as its answer arrives. Not saved.
export function useIntelligenceTests() {
  const [runs, setRuns] = useState({});
  const [selectedTestIds, setSelectedTestIds] = useState(() => INTELLIGENCE_TESTS.map(test => test.id));
  const runIdsRef = useRef({});

  const toggleTestSelected = testId => {
    setSelectedTestIds(prev => (
      prev.includes(testId) ? prev.filter(id => id !== testId) : [...prev, testId]
    ));
  };

  const runTests = (runKey, model, command, send) => {
    const tests = INTELLIGENCE_TESTS.filter(test => selectedTestIds.includes(test.id));
    const runId = (runIdsRef.current[runKey] || 0) + 1;
    runIdsRef.current[runKey] = runId;
    const pending = Object.fromEntries(tests.map(test => [test.id, { loading: true }]));
    setRuns(prev => ({ ...prev, [runKey]: { model, results: pending } }));

    tests.forEach(test => {
      send(test)
        .then(response => ({ response, grade: gradeIntelligenceResponse(test, response) }))
        .catch(err => {
          console.error(`[${command}] ${test.id} failed`, err);
          return {
            response: null,
            grade: { status: 'error', answer: getErrorMessage(err, '测试题发送失败'), detail: '' }
          };
        })
        .then(result => {
          if (runIdsRef.current[runKey] !== runId) return;
          setRuns(prev => {
            const run = prev[runKey];
            return {
              ...prev,
              [runKey]: {
                ...run,
                results: { ...run.results, [test.id]: { loading: false, ...result } }
              }
            };
          });
        });
    });
  };

  const runApiIntelligenceTests = (profile, profileId, model) => {
    runTests(profileId, model, 'test_api_prompt', test => window.api.testApiPrompt({
      baseUrl: profile.base_url || '',
      apiKey: profile.api_key || '',
      model,
      input: test.prompt,
      maxOutputTokens: test.maxOutputTokens
    }));
  };

  const runSubscriptionIntelligenceTests = (accountId, model) => {
    runTests(accountId, model, 'test_subscription_prompt', test => window.api.testSubscriptionPrompt({
      profileId: accountId,
      model,
      input: test.prompt,
      maxOutputTokens: test.maxOutputTokens
    }));
  };

  return {
    intelligenceRuns: runs,
    runApiIntelligenceTests,
    runSubscriptionIntelligenceTests,
    selectedTestIds,
    toggleTestSelected
  };
}
