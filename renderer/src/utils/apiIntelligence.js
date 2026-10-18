// GPT 降智 checks, sent with the precheck's test model. Answers are graded here; the pelican is
// judged by eye. The questions are the ones linux.do uses: the candy puzzle, the pelican variant
// and knowledge questions asked without web search.

const CANDY_PROMPT = [
  '在一个黑色的袋子里放有三种口味的糖果，每种糖果有两种不同的形状（圆形和五角星形，不同的形状靠手感可以分辨）。现已知不同口味的糖和不同形状的数量统计如下表。参赛者需要在活动前决定摸出的糖果数目，那么，最少取出多少个糖果才能保证手中同时拥有不同形状的苹果味和桃子味的糖？（同时手中有圆形苹果味匹配五角星桃子味糖果，或者有圆形桃子味匹配五角星苹果味糖果都满足要求）',
  '',
  '| | 苹果味 | 桃子味 | 西瓜味 |',
  '| --- | --- | --- | --- |',
  '| 圆形 | 7 | 9 | 8 |',
  '| 五角星形 | 7 | 6 | 4 |',
  '',
  '最后一行只写：答案：数字'
].join('\n');

const PELICAN_PROMPT = '做一张精细的 SVG 图片，内容是鹈鹕骑着自行车在孙悟空开的大型飞机机翼上骑行，鹈鹕展开翅膀，上面托着秦始皇的北极熊，秦始皇骑在熊上打螺丝。只输出完整的 SVG 代码，不要输出其他文字。';

function collapse(text, limit = 160) {
  const line = String(text || '').replace(/\s+/g, ' ').trim();
  return line.length > limit ? `${line.slice(0, limit)}…` : line;
}

// The number after the last "答案：", or the last number in the text.
export function finalAnswerNumber(text) {
  const marked = [...String(text || '').matchAll(/答案\s*[:：]\s*(\d+)/g)];
  if (marked.length > 0) return marked[marked.length - 1][1];
  const numbers = String(text || '').match(/\d+/g);
  return numbers ? numbers[numbers.length - 1] : null;
}

export function extractSvg(text) {
  const match = String(text || '').match(/<svg[\s\S]*?<\/svg>/i);
  return match ? match[0] : '';
}

// Shown in an <img>, where an SVG runs no script and loads nothing from outside.
export function svgImageSource(svg) {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

export const INTELLIGENCE_TESTS = [
  {
    id: 'candy',
    title: '糖果题',
    expected: '29',
    maxOutputTokens: 16000,
    prompt: CANDY_PROMPT,
    grade(text) {
      const answer = finalAnswerNumber(text);
      return { status: answer === '29' ? 'pass' : 'fail', answer: answer || collapse(text) };
    }
  },
  {
    id: 'pelican',
    title: '鹈鹕变形',
    expected: '',
    maxOutputTokens: 32000,
    prompt: PELICAN_PROMPT,
    grade(text) {
      const svg = extractSvg(text);
      return svg ? { status: 'visual', answer: '', svg } : { status: 'fail', answer: collapse(text) };
    }
  },
  {
    id: 'thibault',
    title: '知识库：Thibault Sottiaux',
    expected: 'OpenAI（Codex 负责人）',
    maxOutputTokens: 4000,
    prompt: '不搜索网络回答：X 上的 Thibault Sottiaux 是谁？',
    grade(text) {
      return { status: /openai|codex/i.test(text) ? 'pass' : 'fail', answer: collapse(text) };
    }
  },
  {
    id: 'japan-pm',
    title: '知识库：日本首相',
    expected: '高市早苗',
    maxOutputTokens: 4000,
    prompt: '不联网搜索，直接告诉我当今的日本首相是谁',
    grade(text) {
      return { status: String(text || '').includes('高市') ? 'pass' : 'fail', answer: collapse(text) };
    }
  }
];

// What one question's response means: `pass` / `fail` for graded questions, `visual` for the
// pelican, `empty` when the model gave no answer, `error` when the request failed.
export function gradeIntelligenceResponse(test, response) {
  if (!response || response.ok !== true) {
    return { status: 'error', answer: (response && response.error) || '' };
  }
  const text = String(response.outputText || '').trim();
  if (!text) {
    return {
      status: 'empty',
      answer: response.incompleteReason === 'max_output_tokens' ? 'max_output_tokens' : ''
    };
  }
  return test.grade(text);
}

export function intelligenceSummary(results) {
  const graded = INTELLIGENCE_TESTS.filter(test => test.expected);
  const passed = graded.filter(test => results && results[test.id] && results[test.id].grade
    && results[test.id].grade.status === 'pass').length;
  const done = INTELLIGENCE_TESTS.every(test => results && results[test.id] && !results[test.id].loading);
  return { passed, total: graded.length, done };
}
